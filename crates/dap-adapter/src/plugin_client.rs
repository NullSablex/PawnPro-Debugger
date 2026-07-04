//! Cliente do adaptador para o plugin (no servidor) via socket local. Conecta no
//! socket da sessão, envia [`Command`]s e recebe [`Event`]s do plugin numa
//! thread, traduzindo-os em eventos DAP escritos no stdout.

use std::io::{BufRead, BufReader, Write};
use std::sync::{Arc, Mutex};
use std::thread;

use interprocess::local_socket::traits::Stream as _;
use pawnpro_dbg_protocol::transport::{self, LocalStream};
use pawnpro_dbg_protocol::{self as wire, Command, Event};
use serde_json::json;

/// Metade de envio do socket local — para enviar comandos ao plugin.
type SendHalf = <LocalStream as interprocess::local_socket::traits::Stream>::SendHalf;

use crate::messages::Event as DapEvent;
use crate::protocol;

/// Saída DAP compartilhada (stdout) — usada tanto pelo loop principal quanto pela
/// thread que recebe eventos do plugin. O `seq` é monotônico e protegido junto.
#[derive(Clone)]
pub struct DapOut {
    inner: Arc<Mutex<DapOutInner>>,
}

struct DapOutInner {
    seq: i64,
    writer: Box<dyn Write + Send>,
}

impl DapOut {
    pub fn new(writer: Box<dyn Write + Send>, start_seq: i64) -> Self {
        Self {
            inner: Arc::new(Mutex::new(DapOutInner {
                seq: start_seq,
                writer,
            })),
        }
    }

    /// Emite um evento DAP (`stopped`, `terminated`, …) com `seq` próprio.
    pub fn event(&self, event: &'static str, body: serde_json::Value) {
        if let Ok(mut g) = self.inner.lock() {
            g.seq += 1;
            let ev = DapEvent::new(g.seq, event, body);
            if let Ok(s) = serde_json::to_string(&ev) {
                let _ = protocol::write_message(&mut g.writer, &s);
            }
        }
    }

    /// Escreve um corpo DAP já serializado (resposta/evento do loop principal),
    /// com o enquadramento `Content-Length`. Serializa o acesso ao stdout com a
    /// thread de eventos do plugin.
    pub fn write_raw(&self, body: &str) {
        if let Ok(mut g) = self.inner.lock() {
            let _ = protocol::write_message(&mut g.writer, body);
        }
    }
}

/// Estado de escrita do cliente: o lado de envio do socket quando conectado, e
/// uma fila dos comandos recebidos ANTES de conectar (ex.: `setBreakpoints` que
/// chega enquanto o servidor ainda está subindo).
struct Writer {
    send: Option<SendHalf>,
    pending: Vec<String>,
}

/// Conexão com o plugin. A conexão é **assíncrona com retry** numa thread, para
/// não bloquear o loop DAP principal: comandos enviados antes de conectar ficam
/// na fila e são descarregados assim que o socket abre.
pub struct PluginClient {
    writer: Arc<Mutex<Writer>>,
}

impl PluginClient {
    /// Inicia a conexão com o plugin (socket da sessão `id`) numa thread, com
    /// retry. Retorna imediatamente — os comandos enviados nesse meio-tempo são
    /// enfileirados. Emite feedback (`output`/`stopped`/`terminated`) via `out`.
    ///
    /// O servidor, sobretudo com gamemodes grandes, leva vários segundos para
    /// carregar o plugin e abrir o socket; e pode nem ter subido quando o editor
    /// lança o adaptador. Tenta reconectar a cada 200 ms por até ~60 s.
    #[must_use]
    pub fn connect(id: &str, out: DapOut) -> Arc<Self> {
        let writer = Arc::new(Mutex::new(Writer {
            send: None,
            pending: Vec::new(),
        }));
        let client = Arc::new(Self {
            writer: Arc::clone(&writer),
        });

        let id = id.to_string();
        thread::spawn(move || {
            out.event(
                "output",
                json!({ "category": "console", "output": format!("Aguardando o servidor de depuração... (sessão {id})\n") }),
            );
            // --- Retry de conexão (não bloqueia o loop DAP) ---
            let deadline = std::time::Instant::now() + std::time::Duration::from_mins(1);
            let stream = loop {
                match transport::connect(&id) {
                    Ok(s) => break Some(s),
                    Err(_) if std::time::Instant::now() < deadline => {
                        thread::sleep(std::time::Duration::from_millis(200));
                    }
                    Err(e) => {
                        out.event(
                            "output",
                            json!({
                                "category": "stderr",
                                "output": format!(
                                    "Falha ao conectar no plugin de depuração: {e}\n\
                                     Verifique: (1) o servidor está em execução; (2) o plugin do \
                                     PawnPro Debugger está instalado e carregou; (3) não há outro \
                                     plugin com o mesmo nome (que não abre o canal de depuração).\n"
                                ),
                            }),
                        );
                        break None;
                    }
                }
            };
            let Some(stream) = stream else { return };
            let (recv, send) = stream.split();

            // Conectou: instala o `send` e descarrega os comandos enfileirados.
            if let Ok(mut w) = writer.lock() {
                let mut s = send;
                for line in w.pending.drain(..) {
                    let _ = s.write_all(line.as_bytes());
                }
                w.send = Some(s);
            }
            out.event(
                "output",
                json!({ "category": "console", "output": "Conectado ao servidor de depuração.\n" }),
            );

            // --- Loop de leitura de eventos do plugin ---
            let reader = BufReader::new(recv);
            for line in reader.lines() {
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                match wire::from_line::<Event>(&line) {
                    Ok(Event::Paused {
                        reason,
                        line,
                        vars,
                        description,
                    }) => {
                        store_vars(&vars);
                        store_line(line);
                        let mut body = json!({
                            "reason": reason,
                            "threadId": 1,
                            "allThreadsStopped": true,
                            "line": line,
                        });
                        // Runtime error: `description`/`text` show the cause in the
                        // editor's call-stack header (reason "exception").
                        if let Some(desc) = description {
                            body["description"] = json!(desc);
                            body["text"] = json!(desc);
                        }
                        out.event("stopped", body);
                    }
                    Ok(Event::Output { text }) => {
                        // Logpoint: a VM não pausou — só ecoa no console do editor.
                        // `\n` para cada logpoint sair em sua própria linha.
                        out.event(
                            "output",
                            json!({ "category": "console", "output": format!("{text}\n") }),
                        );
                    }
                    Ok(Event::Exited) => {
                        out.event("terminated", serde_json::Value::Null);
                        break;
                    }
                    Err(_) => {} // linha malformada — ignora
                }
            }
        });

        client
    }

    /// Envia um comando ao plugin. Se ainda não conectou, enfileira (será enviado
    /// assim que o socket abrir).
    pub fn send(&self, cmd: &Command) {
        let Ok(line) = wire::to_line(cmd) else { return };
        if let Ok(mut w) = self.writer.lock() {
            if let Some(s) = w.send.as_mut() {
                let _ = s.write_all(line.as_bytes());
            } else {
                w.pending.push(line);
            }
        }
    }
}

/// Últimas variáveis recebidas num `Paused` — servidas ao `variables` do DAP.
/// Global porque chegam pela thread leitora e são consultadas no loop principal.
static LAST_VARS: Mutex<Vec<wire::Var>> = Mutex::new(Vec::new());

fn store_vars(vars: &[wire::Var]) {
    if let Ok(mut g) = LAST_VARS.lock() {
        *g = vars.to_vec();
    }
}

/// Variáveis da última pausa (para o handler `variables` do DAP).
pub fn last_vars() -> Vec<wire::Var> {
    LAST_VARS.lock().map(|g| g.clone()).unwrap_or_default()
}

/// Atualiza no cache o valor de uma variável editada via `setVariable`, para que
/// o painel/watch reflitam o novo valor sem reler a VM (o plugin já a escreveu).
pub fn update_var(name: &str, value: &str) {
    if let Ok(mut g) = LAST_VARS.lock()
        && let Some(v) = g.iter_mut().find(|v| v.name == name)
    {
        v.value = value.to_string();
    }
}

/// Linha-fonte da última pausa (para o `stackTrace` do DAP).
static LAST_LINE: Mutex<Option<i32>> = Mutex::new(None);

fn store_line(line: Option<i32>) {
    if let Ok(mut g) = LAST_LINE.lock() {
        *g = line;
    }
}

/// Linha da última pausa, se houver.
pub fn last_line() -> Option<i32> {
    LAST_LINE.lock().ok().and_then(|g| *g)
}
