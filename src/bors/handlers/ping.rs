use std::sync::Arc;

use crate::bors::Comment;
use crate::bors::RepositoryClient;
use crate::bors::RepositoryState;
use crate::github::PullRequest;

pub(super) async fn command_ping<Client: RepositoryClient>(
    repo: Arc<RepositoryState<Client>>,
    pr: &PullRequest,
) -> anyhow::Result<()> {
    repo.client
        .post_comment(pr.number, Comment::new("Pong 🏓!".to_string()))
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use tracing_test::traced_test;

    use crate::tests::event::default_pr_number;
    use crate::tests::mocks::run_test;
    use crate::tests::state::ClientBuilder;

    #[sqlx::test]
    async fn test_ping(pool: sqlx::PgPool) {
        let state = ClientBuilder::default()
            .pool(pool.clone())
            .create_state()
            .await;
        state.comment("@bors ping").await;
        state
            .client()
            .check_comments(default_pr_number(), &["Pong 🏓!"]);
    }

    #[traced_test]
    #[sqlx::test]
    async fn test_ping2(pool: sqlx::PgPool) {
        run_test(pool, |mut tester| async {
            tester.post_comment("@bors ping").await;
            Ok(tester)
        })
        .await;
    }
}
