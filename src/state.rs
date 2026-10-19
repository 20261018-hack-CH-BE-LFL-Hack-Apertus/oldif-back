use crate::{
    config::Config,
    infrastructure::{core::Core, store::Store},
};
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{Mutex, Semaphore, broadcast};
#[derive(Clone)]
pub struct App {
    pub config: Arc<Config>,
    pub store: Store,
    pub core: Core,
    pub client: reqwest::Client,
    pub events: broadcast::Sender<(String, Value)>,
    pub slots: Arc<Semaphore>,
    pub document_slots: Arc<Semaphore>,
    pub event_lock: Arc<Mutex<()>>,
    pub limits: Arc<Mutex<HashMap<String, (u64, u32)>>>,
}
impl App {
    pub fn new(config: Config) -> Result<Self, reqwest::Error> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()?;
        let (events, _) = broadcast::channel(2048);
        Ok(Self {
            store: Store::new(client.clone(), config.db_url.clone(), config.token.clone()),
            core: Core::new(&config),
            slots: Arc::new(Semaphore::new(config.max_active)),
            document_slots: Arc::new(Semaphore::new(1)),
            client,
            config: Arc::new(config),
            events,
            event_lock: Arc::new(Mutex::new(())),
            limits: Arc::new(Mutex::new(HashMap::new())),
        })
    }
}
