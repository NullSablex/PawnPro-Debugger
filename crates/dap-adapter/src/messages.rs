//! Tipos das mensagens DAP. Mantém-se genérico: `arguments`/`body` ficam como
//! `serde_json::Value`, decodificados por comando conforme necessário — evita
//! modelar todo o protocolo de uma vez.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Mensagem recebida do cliente (o editor). DAP usa `type: "request"`.
#[derive(Debug, Deserialize)]
pub struct Request {
    pub seq: i64,
    pub command: String,
    #[serde(default)]
    pub arguments: Value,
}

/// Resposta a um request (`type: "response"`).
#[derive(Debug, Serialize)]
pub struct Response {
    pub seq: i64,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub request_seq: i64,
    pub success: bool,
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub body: Value,
}

impl Response {
    pub fn ok(seq: i64, req: &Request, body: Value) -> Self {
        Self {
            seq,
            kind: "response",
            request_seq: req.seq,
            success: true,
            command: req.command.clone(),
            message: None,
            body,
        }
    }

    #[allow(dead_code)] // usado quando os handlers passarem a falhar explicitamente
    pub fn fail(seq: i64, req: &Request, message: impl Into<String>) -> Self {
        Self {
            seq,
            kind: "response",
            request_seq: req.seq,
            success: false,
            command: req.command.clone(),
            message: Some(message.into()),
            body: Value::Null,
        }
    }
}

/// Evento enviado ao cliente (`type: "event"`), ex.: `stopped`, `terminated`.
#[derive(Debug, Serialize)]
#[allow(clippy::struct_field_names)] // `event` é o nome do campo no protocolo DAP
pub struct Event {
    pub seq: i64,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub event: &'static str,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub body: Value,
}

impl Event {
    pub fn new(seq: i64, event: &'static str, body: Value) -> Self {
        Self {
            seq,
            kind: "event",
            event,
            body,
        }
    }
}
