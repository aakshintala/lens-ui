use crate::client::Client;
use crate::error::Result;
use crate::ids::{SessionId, TerminalId};

/// Typed terminal REST subservice for one session.
pub struct Terminals<'a> {
    client: &'a Client,
    session: SessionId,
}

impl<'a> Terminals<'a> {
    pub(crate) fn new(client: &'a Client, session: SessionId) -> Self {
        Self { client, session }
    }

    pub fn list(&self) -> Result<Vec<TerminalResource>> {
        let _ = (&self.client, &self.session);
        todo!("Task 2")
    }

    pub fn get(&self, _tid: &TerminalId) -> Result<TerminalResource> {
        todo!("Task 2")
    }

    pub fn create(&self, _req: &TerminalCreate) -> Result<TerminalResource> {
        todo!("Task 2")
    }

    pub fn delete(&self, _tid: &TerminalId) -> Result<()> {
        todo!("Task 2")
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct TerminalCreate {
    pub terminal: String,
    pub session_key: String,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct TerminalMetadata {
    #[serde(default)]
    pub terminal_name: Option<String>,
    #[serde(default)]
    pub session_key: Option<String>,
    #[serde(default)]
    pub running: Option<bool>,
    #[serde(default)]
    pub terminal_transport: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub struct TerminalResource {
    pub id: TerminalId,
    pub session_id: SessionId,
    #[serde(default)]
    pub name: Option<String>,
    pub metadata: TerminalMetadata,
}
