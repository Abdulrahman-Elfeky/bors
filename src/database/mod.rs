//! Provides access to the database.
mod client;
pub(crate) mod operations;

use std::fmt::{Display, Formatter};

use axum::async_trait;
use chrono::{DateTime, Utc};

pub use client::PgDbClient;
use sqlx::{postgres::PgTypeInfo, Postgres};

use crate::github::{CommitSha, GithubRepoName, PullRequestNumber};

type PrimaryKey = i32;

/// A unique identifier for a workflow run.
#[derive(Clone, Copy, Debug)]
pub struct RunId(pub u64);

/// Postgres doesn't support unsigned integers.
impl sqlx::Type<sqlx::Postgres> for RunId {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <i64 as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}

impl From<i64> for RunId {
    fn from(value: i64) -> RunId {
        RunId(value as u64)
    }
}

impl Display for RunId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.0, f)
    }
}

impl From<RunId> for octocrab::models::RunId {
    fn from(val: RunId) -> Self {
        octocrab::models::RunId(val.0)
    }
}

impl From<octocrab::models::RunId> for RunId {
    fn from(val: octocrab::models::RunId) -> Self {
        RunId(val.0)
    }
}

/// Status of a GitHub build.
#[derive(Debug, PartialEq)]
pub enum BuildStatus {
    /// The build is still waiting for results.
    Pending,
    /// The build has succeeded.
    Success,
    /// The build has failed.
    Failure,
    /// The build has been manually cancelled by a user.
    Cancelled,
    /// The build ran for too long and was timeouted by the bot.
    Timeouted,
}

impl sqlx::Type<Postgres> for BuildStatus {
    fn type_info() -> PgTypeInfo {
        <String as sqlx::Type<Postgres>>::type_info()
    }
}

impl sqlx::Decode<'_, Postgres> for BuildStatus {
    fn decode(value: sqlx::postgres::PgValueRef<'_>) -> Result<Self, sqlx::error::BoxDynError> {
        // decode by string
        let status = <String as sqlx::Decode<Postgres>>::decode(value)?;
        match status.as_str() {
            "pending" => Ok(BuildStatus::Pending),
            "success" => Ok(BuildStatus::Success),
            "failure" => Ok(BuildStatus::Failure),
            "cancelled" => Ok(BuildStatus::Cancelled),
            "timeouted" => Ok(BuildStatus::Timeouted),
            _ => Err(format!("Invalid build status: {}", status).into()),
        }
    }
}

impl sqlx::Encode<'_, Postgres> for BuildStatus {
    fn encode_by_ref(&self, buf: &mut sqlx::postgres::PgArgumentBuffer) -> sqlx::encode::IsNull {
        let status = match self {
            BuildStatus::Pending => "pending",
            BuildStatus::Success => "success",
            BuildStatus::Failure => "failure",
            BuildStatus::Cancelled => "cancelled",
            BuildStatus::Timeouted => "timeouted",
        };
        <&str as sqlx::Encode<Postgres>>::encode(status, buf)
    }
}

/// Represents a single (merged) commit.
#[derive(Debug, sqlx::Type)]
#[sqlx(type_name = "build")]
pub struct BuildModel {
    pub id: PrimaryKey,
    pub repository: GithubRepoName,
    pub branch: String,
    pub commit_sha: String,
    pub status: BuildStatus,
    pub parent: String,
    pub created_at: DateTime<Utc>,
}

/// Represents a pull request.
#[derive(Debug)]
pub struct PullRequestModel {
    pub id: PrimaryKey,
    pub repository: GithubRepoName,
    pub number: PullRequestNumber,
    pub try_build: Option<BuildModel>,
    pub created_at: DateTime<Utc>,
}

/// Describes whether a workflow is a Github Actions workflow or if it's a job from some external
/// CI.
#[derive(Debug, PartialEq)]
pub enum WorkflowType {
    Github,
    External,
}

impl sqlx::Type<Postgres> for WorkflowType {
    fn type_info() -> PgTypeInfo {
        <String as sqlx::Type<Postgres>>::type_info()
    }
}

impl sqlx::Decode<'_, Postgres> for WorkflowType {
    fn decode(value: sqlx::postgres::PgValueRef<'_>) -> Result<Self, sqlx::error::BoxDynError> {
        // decode by string
        let status = <String as sqlx::Decode<Postgres>>::decode(value)?;
        match status.as_str() {
            "github" => Ok(WorkflowType::Github),
            "external" => Ok(WorkflowType::External),
            _ => Err(format!("Invalid workflow type: {}", status).into()),
        }
    }
}

impl sqlx::Encode<'_, Postgres> for WorkflowType {
    fn encode_by_ref(&self, buf: &mut sqlx::postgres::PgArgumentBuffer) -> sqlx::encode::IsNull {
        let status = match self {
            WorkflowType::Github => "github",
            WorkflowType::External => "external",
        };
        <&str as sqlx::Encode<Postgres>>::encode(status, buf)
    }
}

/// Status of a workflow.
#[derive(Debug, PartialEq)]
pub enum WorkflowStatus {
    /// Workflow is running.
    Pending,
    /// Workflow has succeeded.
    Success,
    /// Workflow has failed.
    Failure,
}

impl sqlx::Type<Postgres> for WorkflowStatus {
    fn type_info() -> PgTypeInfo {
        <String as sqlx::Type<Postgres>>::type_info()
    }
}

impl sqlx::Decode<'_, Postgres> for WorkflowStatus {
    fn decode(value: sqlx::postgres::PgValueRef<'_>) -> Result<Self, sqlx::error::BoxDynError> {
        // decode by string
        let status = <String as sqlx::Decode<Postgres>>::decode(value)?;
        match status.as_str() {
            "pending" => Ok(WorkflowStatus::Pending),
            "success" => Ok(WorkflowStatus::Success),
            "failure" => Ok(WorkflowStatus::Failure),
            _ => Err(format!("Invalid workflow status: {}", status).into()),
        }
    }
}

impl sqlx::Encode<'_, Postgres> for WorkflowStatus {
    fn encode_by_ref(&self, buf: &mut sqlx::postgres::PgArgumentBuffer) -> sqlx::encode::IsNull {
        let status = match self {
            WorkflowStatus::Pending => "pending",
            WorkflowStatus::Success => "success",
            WorkflowStatus::Failure => "failure",
        };
        <&str as sqlx::Encode<Postgres>>::encode(status, buf)
    }
}

/// Represents a workflow run, coming either from Github Actions or from some external CI.
pub struct WorkflowModel {
    pub id: PrimaryKey,
    pub build: BuildModel,
    pub name: String,
    pub url: String,
    pub run_id: RunId,
    pub workflow_type: WorkflowType,
    pub status: WorkflowStatus,
    pub created_at: DateTime<Utc>,
}

/// Provides access to a database.
#[async_trait]
pub trait DbClient: Sync + Send {
    /// Finds a Pull request row for the given repository and PR number.
    /// If it doesn't exist, a new row is created.
    async fn get_or_create_pull_request(
        &self,
        repo: &GithubRepoName,
        pr_number: PullRequestNumber,
    ) -> anyhow::Result<PullRequestModel>;

    /// Finds a Pull request by a build (either a try or merge one).
    async fn find_pr_by_build(
        &self,
        build: &BuildModel,
    ) -> anyhow::Result<Option<PullRequestModel>>;

    /// Attaches an existing build to the given PR.
    async fn attach_try_build(
        &self,
        pr: PullRequestModel,
        branch: String,
        commit_sha: CommitSha,
        parent: CommitSha,
    ) -> anyhow::Result<()>;

    /// Finds a build row by its repository, commit SHA and branch.
    async fn find_build(
        &self,
        repo: &GithubRepoName,
        branch: String,
        commit_sha: CommitSha,
    ) -> anyhow::Result<Option<BuildModel>>;

    /// Returns all builds that have not been completed yet.
    async fn get_running_builds(&self, repo: &GithubRepoName) -> anyhow::Result<Vec<BuildModel>>;

    /// Updates the status of this build in the DB.
    async fn update_build_status(
        &self,
        build: &BuildModel,
        status: BuildStatus,
    ) -> anyhow::Result<()>;

    /// Creates a new workflow attached to a build.
    async fn create_workflow(
        &self,
        build: &BuildModel,
        name: String,
        url: String,
        run_id: RunId,
        workflow_type: WorkflowType,
        status: WorkflowStatus,
    ) -> anyhow::Result<()>;

    /// Updates the status of a workflow with the given run ID in the DB.
    async fn update_workflow_status(
        &self,
        run_id: u64,
        status: WorkflowStatus,
    ) -> anyhow::Result<()>;

    /// Get all workflows attached to a build.
    async fn get_workflows_for_build(
        &self,
        build: &BuildModel,
    ) -> anyhow::Result<Vec<WorkflowModel>>;
}
