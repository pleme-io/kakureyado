//! SQLite-backed service registry via SeaORM.
//!
//! Gated behind the `persistence` feature. Provides durable storage for onion
//! service metadata so services survive process restarts.

use async_trait::async_trait;
use sea_orm::entity::prelude::*;
use sea_orm::{
    ActiveModelTrait, ActiveValue, ConnectionTrait, Database, DatabaseConnection, Schema,
};

use kakureyado_core::{Error, OnionService, Result, ServiceRegistry, ServiceStatus};

// ---------------------------------------------------------------------------
// Entity
// ---------------------------------------------------------------------------

pub mod onion_service {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "onion_services")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub name: String,
        pub onion_address: String,
        pub target_addr: String,
        pub target_port: i32,
        pub onion_port: i32,
        pub status: String,
        pub persistent: bool,
        pub created_at: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub use onion_service::Entity;

// ---------------------------------------------------------------------------
// Conversions
// ---------------------------------------------------------------------------

impl From<onion_service::Model> for OnionService {
    fn from(m: onion_service::Model) -> Self {
        let status = match m.status.as_str() {
            "running" => ServiceStatus::Running,
            "starting" => ServiceStatus::Starting,
            "error" => ServiceStatus::Error,
            _ => ServiceStatus::Stopped,
        };
        Self {
            name: m.name,
            onion_address: m.onion_address,
            target_addr: m.target_addr,
            target_port: m.target_port as u16,
            status,
            created_at: chrono::DateTime::parse_from_rfc3339(&m.created_at)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .unwrap_or_else(|_| chrono::Utc::now()),
        }
    }
}

fn service_to_active_model(service: &OnionService) -> onion_service::ActiveModel {
    onion_service::ActiveModel {
        id: ActiveValue::Set(service.name.clone()),
        name: ActiveValue::Set(service.name.clone()),
        onion_address: ActiveValue::Set(service.onion_address.clone()),
        target_addr: ActiveValue::Set(service.target_addr.clone()),
        target_port: ActiveValue::Set(i32::from(service.target_port)),
        onion_port: ActiveValue::Set(0),
        status: ActiveValue::Set(service.status.to_string()),
        persistent: ActiveValue::Set(true),
        created_at: ActiveValue::Set(service.created_at.to_rfc3339()),
    }
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// SQLite-backed [`ServiceRegistry`] using SeaORM.
pub struct SqliteRegistry {
    db: DatabaseConnection,
}

impl SqliteRegistry {
    /// Open (or create) an SQLite database and run migrations.
    pub async fn new(db: DatabaseConnection) -> Result<Self> {
        let registry = Self { db };
        registry.create_table_if_needed().await?;
        Ok(registry)
    }

    /// Connect to an SQLite database by URL and run migrations.
    pub async fn connect(url: &str) -> Result<Self> {
        let db = Database::connect(url)
            .await
            .map_err(|e| Error::Config(format!("database connect failed: {e}")))?;
        Self::new(db).await
    }

    async fn create_table_if_needed(&self) -> Result<()> {
        let builder = self.db.get_database_backend();
        let schema = Schema::new(builder);
        let stmt = builder.build(
            schema
                .create_table_from_entity(Entity)
                .if_not_exists(),
        );
        self.db
            .execute(stmt)
            .await
            .map_err(|e| Error::Config(format!("migration failed: {e}")))?;
        Ok(())
    }
}

#[async_trait]
impl ServiceRegistry for SqliteRegistry {
    async fn list(&self) -> Result<Vec<OnionService>> {
        let models = Entity::find()
            .all(&self.db)
            .await
            .map_err(|e| Error::Config(format!("list query failed: {e}")))?;
        Ok(models.into_iter().map(OnionService::from).collect())
    }

    async fn get(&self, name: &str) -> Result<OnionService> {
        Entity::find()
            .filter(onion_service::Column::Name.eq(name))
            .one(&self.db)
            .await
            .map_err(|e| Error::Config(format!("get query failed: {e}")))?
            .map(OnionService::from)
            .ok_or_else(|| Error::ServiceNotFound(name.to_owned()))
    }

    async fn register(&self, service: OnionService) -> Result<()> {
        // Check for duplicates first.
        let existing = Entity::find()
            .filter(onion_service::Column::Name.eq(&service.name))
            .one(&self.db)
            .await
            .map_err(|e| Error::Config(format!("register query failed: {e}")))?;

        if existing.is_some() {
            return Err(Error::AlreadyExists(service.name));
        }

        let active = service_to_active_model(&service);
        active
            .insert(&self.db)
            .await
            .map_err(|e| Error::Config(format!("register insert failed: {e}")))?;
        Ok(())
    }

    async fn unregister(&self, name: &str) -> Result<()> {
        let result = Entity::delete_many()
            .filter(onion_service::Column::Name.eq(name))
            .exec(&self.db)
            .await
            .map_err(|e| Error::Config(format!("unregister delete failed: {e}")))?;

        if result.rows_affected == 0 {
            return Err(Error::ServiceNotFound(name.to_owned()));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use kakureyado_core::ServiceStatus;

    use super::*;

    async fn in_memory_registry() -> SqliteRegistry {
        SqliteRegistry::connect("sqlite::memory:")
            .await
            .expect("in-memory SQLite must succeed")
    }

    fn test_service(name: &str) -> OnionService {
        OnionService {
            name: name.to_owned(),
            onion_address: format!("{name}aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.onion"),
            target_addr: "127.0.0.1".to_owned(),
            target_port: 8080,
            status: ServiceStatus::Stopped,
            created_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn register_and_query() {
        let registry = in_memory_registry().await;
        let svc = test_service("web");

        registry.register(svc).await.expect("register must succeed");

        let found = registry.get("web").await.expect("get must succeed");
        assert_eq!(found.name, "web");
        assert_eq!(found.target_port, 8080);
        assert_eq!(found.status, ServiceStatus::Stopped);
    }

    #[tokio::test]
    async fn list_and_unregister() {
        let registry = in_memory_registry().await;

        registry
            .register(test_service("alpha"))
            .await
            .expect("register alpha");
        registry
            .register(test_service("beta"))
            .await
            .expect("register beta");

        let all = registry.list().await.expect("list must succeed");
        assert_eq!(all.len(), 2);

        registry
            .unregister("alpha")
            .await
            .expect("unregister must succeed");

        let after = registry.list().await.expect("list after unregister");
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].name, "beta");
    }

    #[tokio::test]
    async fn duplicate_rejected() {
        let registry = in_memory_registry().await;
        registry
            .register(test_service("dup"))
            .await
            .expect("first register");

        let err = registry
            .register(test_service("dup"))
            .await
            .expect_err("duplicate must fail");
        assert!(
            matches!(err, Error::AlreadyExists(ref n) if n == "dup"),
            "expected AlreadyExists, got {err:?}"
        );
    }

    #[tokio::test]
    async fn unregister_missing_errors() {
        let registry = in_memory_registry().await;
        let err = registry
            .unregister("ghost")
            .await
            .expect_err("missing unregister must fail");
        assert!(matches!(err, Error::ServiceNotFound(_)));
    }
}
