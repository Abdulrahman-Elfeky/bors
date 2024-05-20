use axum::async_trait;
use sqlx::PgPool;

use crate::database::{
    BuildModel, BuildStatus, PullRequestModel, WorkflowModel, WorkflowStatus, WorkflowType,
};
use crate::github::PullRequestNumber;
use crate::github::{CommitSha, GithubRepoName};

use super::operations::{
    create_build, create_pull_request, create_workflow, find_build, find_pr_by_build,
    get_pull_request, get_running_builds, get_workflows_for_build, update_build_status,
    update_pr_build_id, update_workflow_status,
};
use super::{DbClient, RunId};

/// Provides access to a database using sqlx operations.
#[derive(Clone)]
pub struct PgDbClient {
    pool: PgPool,
}

impl PgDbClient {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl DbClient for PgDbClient {
    async fn get_or_create_pull_request(
        &self,
        repo: &GithubRepoName,
        pr_number: PullRequestNumber,
    ) -> anyhow::Result<PullRequestModel> {
        if let Some(pr) = get_pull_request(&self.pool, repo, pr_number).await? {
            return Ok(pr);
        }
        println!("Creating PR");
        create_pull_request(&self.pool, repo, pr_number).await?;
        let pr = get_pull_request(&self.pool, repo, pr_number)
            .await?
            .expect("PR not found after creation");

        Ok(pr)
    }

    async fn find_pr_by_build(
        &self,
        build: &BuildModel,
    ) -> anyhow::Result<Option<PullRequestModel>> {
        find_pr_by_build(&self.pool, build.id).await
    }

    async fn attach_try_build(
        &self,
        pr: PullRequestModel,
        branch: String,
        commit_sha: CommitSha,
        parent: CommitSha,
    ) -> anyhow::Result<()> {
        let mut tx = self.pool.begin().await?;
        let build_id =
            create_build(&mut *tx, &pr.repository, &branch, &commit_sha, &parent).await?;
        update_pr_build_id(&mut *tx, pr.id, build_id).await?;
        tx.commit().await?;
        Ok(())
    }

    async fn find_build(
        &self,
        repo: &GithubRepoName,
        branch: String,
        commit_sha: CommitSha,
    ) -> anyhow::Result<Option<BuildModel>> {
        find_build(&self.pool, repo, &branch, &commit_sha).await
    }

    async fn get_running_builds(&self, repo: &GithubRepoName) -> anyhow::Result<Vec<BuildModel>> {
        get_running_builds(&self.pool, repo).await
    }

    async fn update_build_status(
        &self,
        build: &BuildModel,
        status: BuildStatus,
    ) -> anyhow::Result<()> {
        update_build_status(&self.pool, build.id, status).await
    }

    async fn create_workflow(
        &self,
        build: &BuildModel,
        name: String,
        url: String,
        run_id: RunId,
        workflow_type: WorkflowType,
        status: WorkflowStatus,
    ) -> anyhow::Result<()> {
        create_workflow(
            &self.pool,
            build.id,
            &name,
            &url,
            run_id,
            workflow_type,
            status,
        )
        .await
    }

    async fn update_workflow_status(
        &self,
        run_id: u64,
        status: WorkflowStatus,
    ) -> anyhow::Result<()> {
        update_workflow_status(&self.pool, run_id, status).await
    }

    async fn get_workflows_for_build(
        &self,
        build: &BuildModel,
    ) -> anyhow::Result<Vec<WorkflowModel>> {
        get_workflows_for_build(&self.pool, build.id).await
    }
}
