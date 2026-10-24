use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgConnection};
use uuid::Uuid;

use crate::{Result, Store};

#[derive(Debug, Clone, FromRow, PartialEq)]
pub struct UserRow {
    pub id: Uuid,
    pub github_id: i64,
    pub login: String,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Clone, FromRow)]
pub struct TokenRow {
    pub user_id: Uuid,
    pub access_token: Vec<u8>,
    pub access_expires_at: Option<DateTime<Utc>>,
    pub refresh_token: Option<Vec<u8>>,
    pub refresh_expires_at: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for TokenRow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenRow")
            .field("user_id", &self.user_id)
            .field("access_expires_at", &self.access_expires_at)
            .field("refresh_expires_at", &self.refresh_expires_at)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub struct SessionUser {
    pub user: UserRow,
    pub expires_at: DateTime<Utc>,
}

impl Store {
    pub async fn upsert_user(
        &self,
        github_id: i64,
        login: &str,
        name: Option<&str>,
        avatar_url: Option<&str>,
    ) -> Result<UserRow> {
        let row = sqlx::query_as::<_, UserRow>(
            r#"
            INSERT INTO users (id, github_id, login, name, avatar_url, last_login_at)
            VALUES ($1, $2, $3, $4, $5, now())
            ON CONFLICT (github_id) DO UPDATE
               SET login = EXCLUDED.login, name = EXCLUDED.name, avatar_url = EXCLUDED.avatar_url,
                   last_login_at = now(), updated_at = now()
            RETURNING id, github_id, login, name, avatar_url
            "#,
        )
        .bind(Uuid::new_v4())
        .bind(github_id)
        .bind(login)
        .bind(name)
        .bind(avatar_url)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn get_user(&self, id: Uuid) -> Result<Option<UserRow>> {
        Ok(sqlx::query_as::<_, UserRow>("SELECT id, github_id, login, name, avatar_url FROM users WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?)
    }

    pub async fn store_tokens(
        &self,
        user_id: Uuid,
        access_token: &[u8],
        access_expires_at: Option<DateTime<Utc>>,
        refresh_token: Option<&[u8]>,
        refresh_expires_at: Option<DateTime<Utc>>,
    ) -> Result<()> {
        let mut conn = self.pool.acquire().await?;
        store_tokens_on(&mut conn, user_id, access_token, access_expires_at, refresh_token, refresh_expires_at).await
    }

    pub async fn get_tokens(&self, user_id: Uuid) -> Result<Option<TokenRow>> {
        Ok(sqlx::query_as::<_, TokenRow>(
            "SELECT user_id, access_token, access_expires_at, refresh_token, refresh_expires_at
               FROM github_tokens WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn delete_tokens(&self, user_id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM github_tokens WHERE user_id = $1").bind(user_id).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn create_session(&self, id_hash: &[u8], user_id: Uuid, expires_at: DateTime<Utc>) -> Result<()> {
        sqlx::query("INSERT INTO sessions (id_hash, user_id, expires_at) VALUES ($1, $2, $3)")
            .bind(id_hash)
            .bind(user_id)
            .bind(expires_at)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Resolves a live session and records activity (at most once a minute to limit writes).
    pub async fn session_user(&self, id_hash: &[u8]) -> Result<Option<SessionUser>> {
        #[derive(FromRow)]
        struct Row {
            id: Uuid,
            github_id: i64,
            login: String,
            name: Option<String>,
            avatar_url: Option<String>,
            expires_at: DateTime<Utc>,
            stale: bool,
        }
        let row = sqlx::query_as::<_, Row>(
            r#"
            SELECT u.id, u.github_id, u.login, u.name, u.avatar_url, s.expires_at,
                   s.last_seen_at < now() - interval '60 seconds' AS stale
              FROM sessions s JOIN users u ON u.id = s.user_id
             WHERE s.id_hash = $1 AND s.expires_at > now()
            "#,
        )
        .bind(id_hash)
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else { return Ok(None) };
        if row.stale {
            sqlx::query("UPDATE sessions SET last_seen_at = now() WHERE id_hash = $1")
                .bind(id_hash)
                .execute(&self.pool)
                .await?;
        }
        Ok(Some(SessionUser {
            user: UserRow {
                id: row.id,
                github_id: row.github_id,
                login: row.login,
                name: row.name,
                avatar_url: row.avatar_url,
            },
            expires_at: row.expires_at,
        }))
    }

    pub async fn delete_session(&self, id_hash: &[u8]) -> Result<()> {
        sqlx::query("DELETE FROM sessions WHERE id_hash = $1").bind(id_hash).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn purge_expired_sessions(&self) -> Result<u64> {
        let r = sqlx::query("DELETE FROM sessions WHERE expires_at < now()").execute(&self.pool).await?;
        Ok(r.rows_affected())
    }
}

/// Locks the user's token row for a refresh. GitHub refresh tokens are single-use, so concurrent refreshes
/// (API pods, workers) must serialise on this lock.
pub async fn tokens_for_update(conn: &mut PgConnection, user_id: Uuid) -> Result<Option<TokenRow>> {
    Ok(sqlx::query_as::<_, TokenRow>(
        "SELECT user_id, access_token, access_expires_at, refresh_token, refresh_expires_at
           FROM github_tokens WHERE user_id = $1 FOR UPDATE",
    )
    .bind(user_id)
    .fetch_optional(conn)
    .await?)
}

pub async fn store_tokens_on(
    conn: &mut PgConnection,
    user_id: Uuid,
    access_token: &[u8],
    access_expires_at: Option<DateTime<Utc>>,
    refresh_token: Option<&[u8]>,
    refresh_expires_at: Option<DateTime<Utc>>,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO github_tokens (user_id, access_token, access_expires_at, refresh_token, refresh_expires_at)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (user_id) DO UPDATE
           SET access_token = EXCLUDED.access_token, access_expires_at = EXCLUDED.access_expires_at,
               refresh_token = EXCLUDED.refresh_token, refresh_expires_at = EXCLUDED.refresh_expires_at,
               updated_at = now()
        "#,
    )
    .bind(user_id)
    .bind(access_token)
    .bind(access_expires_at)
    .bind(refresh_token)
    .bind(refresh_expires_at)
    .execute(conn)
    .await?;
    Ok(())
}
