use crate::PgDbClient;
use crate::bors::event::{PullRequestEdited, PullRequestOpened, PullRequestPushed, PushToBranch};
use crate::bors::handlers::labels::handle_label_trigger;
use crate::bors::{Comment, RepositoryState};
use crate::database::MergeableState;
use crate::github::{CommitSha, LabelTrigger, PullRequestNumber};
use std::sync::Arc;

pub(super) async fn handle_pull_request_edited(
    repo_state: Arc<RepositoryState>,
    db: Arc<PgDbClient>,
    payload: PullRequestEdited,
) -> anyhow::Result<()> {
    let pr = &payload.pull_request;
    let pr_number = pr.number;
    let pr_model = db
        .get_or_create_pull_request(
            repo_state.repository(),
            pr_number,
            &pr.base.name,
            pr.mergeable_state.clone().into(),
        )
        .await?;

    // If the base branch has changed, unapprove the PR
    let Some(_) = payload.from_base_sha else {
        return Ok(());
    };

    if !pr_model.is_approved() {
        return Ok(());
    }

    db.unapprove(&pr_model).await?;
    handle_label_trigger(&repo_state, pr_number, LabelTrigger::Unapproved).await?;
    notify_of_edited_pr(&repo_state, pr_number, &payload.pull_request.base.name).await
}

pub(super) async fn handle_push_to_pull_request(
    repo_state: Arc<RepositoryState>,
    db: Arc<PgDbClient>,
    payload: PullRequestPushed,
) -> anyhow::Result<()> {
    let pr = &payload.pull_request;
    let pr_number = pr.number;
    let pr_model = db
        .get_or_create_pull_request(
            repo_state.repository(),
            pr_number,
            &pr.base.name,
            pr.mergeable_state.clone().into(),
        )
        .await?;

    if !pr_model.is_approved() {
        return Ok(());
    }

    db.unapprove(&pr_model).await?;
    handle_label_trigger(&repo_state, pr_number, LabelTrigger::Unapproved).await?;
    notify_of_pushed_pr(&repo_state, pr_number, pr.head.sha.clone()).await
}

pub(super) async fn handle_pull_request_opened(
    repo_state: Arc<RepositoryState>,
    db: Arc<PgDbClient>,
    payload: PullRequestOpened,
) -> anyhow::Result<()> {
    db.create_pull_request(
        repo_state.repository(),
        payload.pull_request.number,
        &payload.pull_request.base.name,
    )
    .await
}

pub(super) async fn handle_push_to_branch(
    repo_state: Arc<RepositoryState>,
    db: Arc<PgDbClient>,
    payload: PushToBranch,
) -> anyhow::Result<()> {
    let rows = db
        .update_mergeable_states_by_base_branch(
            repo_state.repository(),
            &payload.branch,
            MergeableState::Unknown,
        )
        .await?;

    tracing::info!("Updated mergeable_state to `unknown` for {} PR(s)", rows);

    Ok(())
}

async fn notify_of_edited_pr(
    repo: &RepositoryState,
    pr_number: PullRequestNumber,
    base_name: &str,
) -> anyhow::Result<()> {
    repo.client
        .post_comment(
            pr_number,
            Comment::new(format!(
                r#":warning: The base branch changed to `{base_name}`, and the
PR will need to be re-approved."#,
            )),
        )
        .await
}

async fn notify_of_pushed_pr(
    repo: &RepositoryState,
    pr_number: PullRequestNumber,
    head_sha: CommitSha,
) -> anyhow::Result<()> {
    repo.client
        .post_comment(
            pr_number,
            Comment::new(format!(
                r#":warning: A new commit `{}` was pushed to the branch, the
PR will need to be re-approved."#,
                head_sha
            )),
        )
        .await
}

#[cfg(test)]
mod tests {
    use crate::tests::mocks::default_pr_number;
    use crate::{
        database::MergeableState,
        tests::mocks::{User, default_branch_name, default_repo_name, run_test},
    };

    #[sqlx::test]
    async fn unapprove_on_base_edited(pool: sqlx::PgPool) {
        run_test(pool, |mut tester| async {
            tester.post_comment("@bors r+").await?;
            tester.expect_comments(1).await;
            let branch = tester.create_branch("beta").clone();
            tester
                .edit_pr(default_repo_name(), default_pr_number(), |pr| {
                    pr.base_branch = branch;
                })
                .await?;

            insta::assert_snapshot!(
                tester.get_comment().await?,
                @r"
            :warning: The base branch changed to `beta`, and the
            PR will need to be re-approved.
            "
            );
            tester.default_pr().await.expect_unapproved();
            Ok(tester)
        })
        .await;
    }

    #[sqlx::test]
    async fn edit_pr_do_nothing_when_base_not_edited(pool: sqlx::PgPool) {
        run_test(pool, |mut tester| async {
            tester.post_comment("@bors r+").await?;
            tester.expect_comments(1).await;
            tester
                .edit_pr(default_repo_name(), default_pr_number(), |_| {})
                .await?;

            tester
                .default_pr()
                .await
                .expect_approved_by(&User::default_pr_author().name);
            Ok(tester)
        })
        .await;
    }

    #[sqlx::test]
    async fn edit_pr_do_nothing_when_not_approved(pool: sqlx::PgPool) {
        run_test(pool, |mut tester| async {
            let branch = tester.create_branch("beta").clone();
            tester
                .edit_pr(default_repo_name(), default_pr_number(), |pr| {
                    pr.base_branch = branch;
                })
                .await?;

            // No comment should be posted
            Ok(tester)
        })
        .await;
    }

    #[sqlx::test]
    async fn unapprove_on_push(pool: sqlx::PgPool) {
        run_test(pool, |mut tester| async {
            tester.post_comment("@bors r+").await?;
            tester.expect_comments(1).await;
            tester
                .push_to_pr(default_repo_name(), default_pr_number())
                .await?;

            insta::assert_snapshot!(
                tester.get_comment().await?,
                @r"
            :warning: A new commit `pr-1-commit-1` was pushed to the branch, the
            PR will need to be re-approved.
            "
            );
            tester.default_pr().await.expect_unapproved();
            Ok(tester)
        })
        .await;
    }

    #[sqlx::test]
    async fn push_to_pr_do_nothing_when_not_approved(pool: sqlx::PgPool) {
        run_test(pool, |mut tester| async {
            tester
                .push_to_pr(default_repo_name(), default_pr_number())
                .await?;

            // No comment should be posted
            Ok(tester)
        })
        .await;
    }

    #[sqlx::test]
    async fn store_base_branch_on_pr_opened(pool: sqlx::PgPool) {
        run_test(pool, |mut tester| async {
            let pr = tester.open_pr(default_repo_name()).await?;
            tester
                .wait_for(|| async {
                    let Some(pr) = tester.pr_db(default_repo_name(), pr.number.0).await? else {
                        return Ok(false);
                    };
                    Ok(pr.base_branch == *default_branch_name())
                })
                .await?;
            Ok(tester)
        })
        .await;
    }

    #[sqlx::test]
    async fn update_base_branch_on_pr_edited(pool: sqlx::PgPool) {
        run_test(pool.clone(), |mut tester| async {
            let branch = tester.create_branch("foo").clone();
            tester
                .edit_pr(default_repo_name(), default_pr_number(), |pr| {
                    pr.base_branch = branch;
                })
                .await?;
            tester
                .wait_for(|| async {
                    let Some(pr) = tester.default_pr_db().await? else {
                        return Ok(false);
                    };
                    Ok(pr.base_branch == "foo")
                })
                .await?;
            Ok(tester)
        })
        .await;
    }

    #[sqlx::test]
    async fn update_mergeable_state_on_pr_edited(pool: sqlx::PgPool) {
        run_test(pool.clone(), |mut tester| async {
            tester
                .edit_pr(default_repo_name(), default_pr_number(), |pr| {
                    pr.mergeable_state = octocrab::models::pulls::MergeableState::Dirty;
                })
                .await?;
            tester
                .wait_for(|| async {
                    let Some(pr) = tester.default_pr_db().await? else {
                        return Ok(false);
                    };
                    Ok(pr.mergeable_state == MergeableState::HasConflicts)
                })
                .await?;
            Ok(tester)
        })
        .await;
    }
}
