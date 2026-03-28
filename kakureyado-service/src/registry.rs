use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use kakureyado_core::{Error, OnionService, Result, ServiceRegistry};

/// In-memory service registry for testing and development.
pub struct MemoryRegistry {
    services: Arc<RwLock<HashMap<String, OnionService>>>,
}

impl MemoryRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            services: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl Default for MemoryRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ServiceRegistry for MemoryRegistry {
    async fn list(&self) -> Result<Vec<OnionService>> {
        let services = self.services.read().await;
        Ok(services.values().cloned().collect())
    }

    async fn get(&self, name: &str) -> Result<OnionService> {
        let services = self.services.read().await;
        services
            .get(name)
            .cloned()
            .ok_or_else(|| Error::ServiceNotFound(name.to_owned()))
    }

    async fn register(&self, service: OnionService) -> Result<()> {
        let mut services = self.services.write().await;
        if services.contains_key(&service.name) {
            return Err(Error::AlreadyExists(service.name));
        }
        services.insert(service.name.clone(), service);
        Ok(())
    }

    async fn unregister(&self, name: &str) -> Result<()> {
        let mut services = self.services.write().await;
        services
            .remove(name)
            .ok_or_else(|| Error::ServiceNotFound(name.to_owned()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use kakureyado_core::ServiceStatus;

    use super::*;

    fn test_service(name: &str) -> OnionService {
        OnionService {
            name: name.to_owned(),
            onion_address: format!("{name}.onion"),
            target_addr: "127.0.0.1".to_owned(),
            target_port: 8080,
            status: ServiceStatus::Stopped,
            created_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn register_and_get() {
        let registry = MemoryRegistry::new();
        registry
            .register(test_service("web"))
            .await
            .expect("register must succeed");

        let svc = registry.get("web").await.expect("get must succeed");
        assert_eq!(svc.name, "web");
    }

    #[tokio::test]
    async fn list_returns_all() {
        let registry = MemoryRegistry::new();
        registry
            .register(test_service("a"))
            .await
            .expect("register a");
        registry
            .register(test_service("b"))
            .await
            .expect("register b");

        let all = registry.list().await.expect("list must succeed");
        assert_eq!(all.len(), 2);
    }

    #[tokio::test]
    async fn unregister_removes() {
        let registry = MemoryRegistry::new();
        registry
            .register(test_service("gone"))
            .await
            .expect("register");
        registry.unregister("gone").await.expect("unregister");

        let err = registry
            .get("gone")
            .await
            .expect_err("get after unregister must fail");
        assert!(matches!(err, Error::ServiceNotFound(_)));
    }
}
