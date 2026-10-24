//! Generator repositories saved by users (the operator-curated ones live in configuration, not here).

use chrono::{DateTime, Utc};
use sqlx::FromRow;
use uuid::Uuid;

use crate::{Result, Store, StoreError};

#[derive(Debug, Clone, FromRow)]
pub struct UserGeneratorRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub owner: String,
    pub repo: String,
    pub branch: Option<String>,
    pub path: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewUserGenerator<'a> {
    pub user_id: Uuid,
    pub name: &'a str,
    pub owner: &'a str,
    pub repo: &'a str,
    pub branch: Option<&'a str>,
    pub path: &'a str,
}

const COLUMNS: &str = "id, user_id, name, owner, repo, branch, path, created_at";

impl Store {
    pub async fn list_user_generators(&self, user_id: Uuid) -> Result<Vec<UserGeneratorRow>> {
        Ok(sqlx::query_as::<_, UserGeneratorRow>(&format!(
            "SELECT {COLUMNS} FROM user_generators WHERE user_id = $1 ORDER BY lower(name), created_at"
        ))
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn get_user_generator(&self, user_id: Uuid, id: Uuid) -> Result<UserGeneratorRow> {
        sqlx::query_as::<_, UserGeneratorRow>(&format!(
            "SELECT {COLUMNS} FROM user_generators WHERE id = $1 AND user_id = $2"
        ))
        .bind(id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(StoreError::NotFound)
    }

    /// Saves a generator repository. Adding the same repository/branch/path again returns the existing row.
    pub async fn add_user_generator(&self, new: NewUserGenerator<'_>) -> Result<UserGeneratorRow> {
        let inserted = sqlx::query_as::<_, UserGeneratorRow>(&format!(
            "INSERT INTO user_generators (id, user_id, name, owner, repo, branch, path)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT DO NOTHING
             RETURNING {COLUMNS}"
        ))
        .bind(Uuid::new_v4())
        .bind(new.user_id)
        .bind(new.name)
        .bind(new.owner)
        .bind(new.repo)
        .bind(new.branch)
        .bind(new.path)
        .fetch_optional(&self.pool)
        .await?;
        match inserted {
            Some(row) => Ok(row),
            None => Ok(sqlx::query_as::<_, UserGeneratorRow>(&format!(
                "SELECT {COLUMNS} FROM user_generators
                  WHERE user_id = $1 AND lower(owner) = lower($2) AND lower(repo) = lower($3)
                    AND coalesce(branch, '') = coalesce($4, '') AND path = $5"
            ))
            .bind(new.user_id)
            .bind(new.owner)
            .bind(new.repo)
            .bind(new.branch)
            .bind(new.path)
            .fetch_one(&self.pool)
            .await?),
        }
    }

    pub async fn delete_user_generator(&self, user_id: Uuid, id: Uuid) -> Result<()> {
        let r = sqlx::query("DELETE FROM user_generators WHERE id = $1 AND user_id = $2")
            .bind(id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        if r.rows_affected() == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }

    /// Cap on saved generators per user (checked by the API before inserting).
    pub async fn count_user_generators(&self, user_id: Uuid) -> Result<i64> {
        Ok(sqlx::query_scalar("SELECT count(*) FROM user_generators WHERE user_id = $1")
            .bind(user_id)
            .fetch_one(&self.pool)
            .await?)
    }
}
