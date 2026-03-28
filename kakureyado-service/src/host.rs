use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use tokio::sync::RwLock;

use kakureyado_core::{
    Error, OnionService, OnionServiceHost, Result, ServiceConfig, ServiceStatus,
};

/// In-memory onion service host for development and testing.
///
/// Manages service state without an actual Tor process — useful for integration
/// tests and as a reference implementation of the [`OnionServiceHost`] trait.
pub struct LocalOnionHost {
    services: Arc<RwLock<HashMap<String, OnionService>>>,
}

impl LocalOnionHost {
    #[must_use]
    pub fn new() -> Self {
        Self {
            services: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

impl Default for LocalOnionHost {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl OnionServiceHost for LocalOnionHost {
    async fn create_service(&self, config: &ServiceConfig) -> Result<OnionService> {
        let mut services = self.services.write().await;

        if services.contains_key(&config.name) {
            return Err(Error::AlreadyExists(config.name.clone()));
        }

        let service = OnionService {
            name: config.name.clone(),
            onion_address: format!("{}.onion", config.name),
            target_addr: config.target_addr.clone(),
            target_port: config.target_port,
            status: ServiceStatus::Stopped,
            created_at: Utc::now(),
        };

        services.insert(config.name.clone(), service.clone());
        Ok(service)
    }

    async fn start_service(&self, name: &str) -> Result<()> {
        let mut services = self.services.write().await;
        let service = services
            .get_mut(name)
            .ok_or_else(|| Error::ServiceNotFound(name.to_owned()))?;
        service.status = ServiceStatus::Running;
        Ok(())
    }

    async fn stop_service(&self, name: &str) -> Result<()> {
        let mut services = self.services.write().await;
        let service = services
            .get_mut(name)
            .ok_or_else(|| Error::ServiceNotFound(name.to_owned()))?;
        service.status = ServiceStatus::Stopped;
        Ok(())
    }

    async fn service_status(&self, name: &str) -> Result<ServiceStatus> {
        let services = self.services.read().await;
        services
            .get(name)
            .map(|s| s.status)
            .ok_or_else(|| Error::ServiceNotFound(name.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config(name: &str) -> ServiceConfig {
        ServiceConfig {
            name: name.to_owned(),
            target_addr: "127.0.0.1".to_owned(),
            target_port: 8080,
            onion_port: 80,
            persistent: false,
            vanity_prefix: None,
        }
    }

    #[tokio::test]
    async fn create_and_list() {
        let host = LocalOnionHost::new();
        let svc = host
            .create_service(&test_config("web"))
            .await
            .expect("create must succeed");
        assert_eq!(svc.name, "web");
        assert_eq!(svc.status, ServiceStatus::Stopped);

        let status = host
            .service_status("web")
            .await
            .expect("status must succeed");
        assert_eq!(status, ServiceStatus::Stopped);
    }

    #[tokio::test]
    async fn start_stop_lifecycle() {
        let host = LocalOnionHost::new();
        host.create_service(&test_config("api"))
            .await
            .expect("create");

        host.start_service("api").await.expect("start");
        assert_eq!(
            host.service_status("api").await.unwrap(),
            ServiceStatus::Running
        );

        host.stop_service("api").await.expect("stop");
        assert_eq!(
            host.service_status("api").await.unwrap(),
            ServiceStatus::Stopped
        );
    }

    #[tokio::test]
    async fn duplicate_name_rejected() {
        let host = LocalOnionHost::new();
        host.create_service(&test_config("dup"))
            .await
            .expect("first create");

        let err = host
            .create_service(&test_config("dup"))
            .await
            .expect_err("duplicate must fail");
        assert!(
            matches!(err, Error::AlreadyExists(ref n) if n == "dup"),
            "expected AlreadyExists, got {err:?}"
        );
    }
}
