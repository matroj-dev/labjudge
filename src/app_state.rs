use std::sync::{Arc, RwLock};

use crate::auth::csrf::Csrf;
use crate::auth::session::{AuthError, SessionSigner};
use crate::config::Config;
use crate::judge::{Compiler, JudgeQueueHandle, Supervisor};
use crate::paths::Paths;
use crate::services::registry::ContestRegistry;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub paths: Arc<Paths>,
    pub port: u16,
    pub registry: Arc<ContestRegistry>,
    pub session: SessionSigner,
    pub csrf: Csrf,
    pub compiler: Option<Arc<Compiler>>,
    pub supervisor: Option<Arc<Supervisor>>,
    pub queue: Option<JudgeQueueHandle>,
    pub admin_password: Arc<RwLock<String>>,
}

impl AppState {
    pub fn new(
        config: Arc<Config>,
        paths: Arc<Paths>,
        port: u16,
        registry: Arc<ContestRegistry>,
    ) -> Result<Self, AuthError> {
        let session = SessionSigner::from_hex(&config.session_secret)?;
        let csrf = Csrf::from_key(session.key_bytes());
        let admin_password = Arc::new(RwLock::new(config.admin_password.clone()));
        Ok(Self {
            config,
            paths,
            port,
            registry,
            session,
            csrf,
            compiler: None,
            supervisor: None,
            queue: None,
            admin_password,
        })
    }

    pub fn for_test() -> Self {
        let config = Config::for_test();
        let admin_password = Arc::new(RwLock::new(config.admin_password.clone()));
        let session = SessionSigner::from_hex(&config.session_secret).unwrap();
        let csrf = Csrf::from_key(session.key_bytes());
        Self {
            config: Arc::new(config),
            paths: Arc::new(Paths::for_test(std::env::temp_dir().join("labjudge-test"))),
            port: 0,
            registry: Arc::new(ContestRegistry::empty()),
            session,
            csrf,
            compiler: None,
            supervisor: None,
            queue: None,
            admin_password,
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::for_test()
    }
}
