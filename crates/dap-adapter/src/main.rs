//! Adaptador DAP (Componente 3 do debugger) — processo lançado pelo editor.
//! Traduz Debug Adapter Protocol (DAP) ⇄ protocolo do plugin, usando
//! `samp_sdk::debug` para mapear linha ↔ endereço.
//!
//! Loop síncrono sobre stdin/stdout (igual a um LSP básico). Uma thread separada
//! recebe eventos do plugin (socket local) e os escreve como eventos DAP no stdout.

mod messages;
mod plugin_client;
mod protocol;
mod session;

use std::io::{self, BufReader};
use std::process::Child;
use std::sync::Arc;

use messages::Request;
use plugin_client::{DapOut, PluginClient};
use session::{Outgoing, Session, SpawnSpec};

fn main() -> io::Result<()> {
    let stdin = io::stdin();
    let mut reader = BufReader::new(stdin.lock());

    // Saída DAP compartilhada entre o loop principal e a thread de eventos do
    // plugin — ambos escrevem no stdout. O `session` numera suas respostas a
    // partir de 1; os eventos do plugin usam um range alto para não colidir.
    let out = DapOut::new(Box::new(io::stdout()), 1_000_000);
    let mut plugin: Option<Arc<PluginClient>> = None;
    // Servidor do jogo como processo FILHO. `ServerChild` mata o processo no
    // `Drop` — ou seja, quando o adaptador encerra (fim do loop) ou morre.
    let mut server: Option<ServerChild> = None;

    let mut session = Session::new();

    while let Some(raw) = protocol::read_message(&mut reader)? {
        let req: Request = match serde_json::from_str(&raw) {
            Ok(r) => r,
            Err(_) => continue, // malformada — ignora, não derruba
        };
        for outgoing in session.handle(&req) {
            match outgoing {
                Outgoing::Response(_) | Outgoing::Event(_) => emit(&out, &outgoing),
                Outgoing::SpawnServer(spec) => match spawn_server(&spec, &out) {
                    Ok(child) => server = Some(child),
                    Err(e) => out.event(
                        "output",
                        serde_json::json!({
                            "category": "stderr",
                            "output": format!("Falha ao iniciar o servidor: {e}\n"),
                        }),
                    ),
                },
                Outgoing::ConnectPlugin(id) => {
                    // A conexão é assíncrona (com retry e feedback próprios);
                    // não bloqueia o loop nem falha de imediato.
                    plugin = Some(PluginClient::connect(&id, out.clone()));
                }
                Outgoing::ToPlugin(cmd) => {
                    if let Some(c) = &plugin {
                        c.send(&cmd);
                    }
                }
            }
        }
        if session.is_terminated() {
            break;
        }
    }
    // Fim do loop (encerrar/reiniciar): o `Drop` de `server` mata o processo do
    // servidor. Explícito para deixar claro que é aqui que ele cai.
    drop(server);
    Ok(())
}

/// Servidor do jogo como processo filho. Mata-o no `Drop` (encerrar/reiniciar a
/// depuração faz o editor matar o adaptador → este `Drop` roda → servidor cai).
struct ServerChild(Child);

impl Drop for ServerChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Sobe o servidor do jogo com as variáveis de depuração. No Linux, pede ao
/// kernel para matar o filho se o pai (adaptador) morrer abruptamente
/// (`PR_SET_PDEATHSIG`), cobrindo o caso de o editor matar o adaptador sem o
/// `Drop` rodar.
///
/// `stdout`/`stderr` do servidor são capturados (`piped`) e reencaminhados ao
/// console de Depuração do editor como eventos DAP `output` — assim o dev vê os
/// `print`/logs do gamemode sem precisar de um terminal à parte.
fn spawn_server(spec: &SpawnSpec, out: &DapOut) -> io::Result<ServerChild> {
    let mut cmd = std::process::Command::new(&spec.exe);
    cmd.args(&spec.args)
        .env("PAWNPRO_DBG_SESSION", &spec.session)
        .env("PAWNPRO_DBG_AMXDBG", &spec.amx_path)
        .env("PAWNPRO_DBG_LOCALE", &spec.locale)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if !spec.cwd.is_empty() {
        cmd.current_dir(&spec.cwd);
    }
    #[cfg(target_os = "linux")]
    unsafe {
        use std::os::unix::process::CommandExt;
        cmd.pre_exec(|| {
            // SIGKILL (9) no filho se o pai morrer.
            libc_prctl_pdeathsig();
            Ok(())
        });
    }
    let mut child = cmd.spawn()?;

    // Uma thread por fluxo: lê linha a linha e emite `output`. O servidor SA-MP
    // imprime no stdout; erros vão pro stderr (categoria "stderr" deixa o editor
    // colorir diferente). As threads terminam sozinhas no EOF (quando o servidor
    // morre e a pipe fecha) — não há o que limpar.
    if let Some(stdout) = child.stdout.take() {
        forward_stream(stdout, "stdout", out.clone());
    }
    if let Some(stderr) = child.stderr.take() {
        forward_stream(stderr, "stderr", out.clone());
    }

    Ok(ServerChild(child))
}

/// Lê `stream` linha a linha numa thread dedicada e emite cada linha como um
/// evento DAP `output` na categoria dada. Linhas inválidas em UTF-8 são lidas
/// com substituição (servidor pode emitir bytes não-UTF-8); preserva a quebra
/// de linha que o console do editor espera.
fn forward_stream<R: io::Read + Send + 'static>(stream: R, category: &'static str, out: DapOut) {
    std::thread::spawn(move || pump_stream(stream, category, &out));
}

/// Lógica síncrona de `forward_stream`, isolada para ser testável sem thread.
/// Lê `stream` linha a linha e emite cada linha como `output`. `read_until('\n')`
/// (em vez de `lines()`) preserva a quebra de linha e a última linha sem `\n`, e
/// não falha com bytes não-UTF-8 (lidos com substituição).
fn pump_stream<R: io::Read>(stream: R, category: &'static str, out: &DapOut) {
    use std::io::BufRead;
    let mut reader = BufReader::new(stream);
    let mut buf = Vec::new();
    while let Ok(n) = reader.read_until(b'\n', &mut buf) {
        if n == 0 {
            break; // EOF — o servidor fechou a pipe
        }
        let text = String::from_utf8_lossy(&buf).into_owned();
        out.event(
            "output",
            serde_json::json!({ "category": category, "output": text }),
        );
        buf.clear();
    }
}

/// `prctl(PR_SET_PDEATHSIG, SIGKILL)` sem dependência externa (chamada direta).
#[cfg(target_os = "linux")]
fn libc_prctl_pdeathsig() {
    const PR_SET_PDEATHSIG: i32 = 1;
    const SIGKILL: i32 = 9;
    unsafe {
        // syscall prctl (157 em x86_64; usamos a libc via extern).
        unsafe extern "C" {
            fn prctl(option: i32, ...) -> i32;
        }
        prctl(PR_SET_PDEATHSIG, SIGKILL);
    }
}

/// Escreve uma resposta/evento DAP gerado pelo `session` no stdout, usando o
/// `DapOut` para serializar o `seq` de forma consistente com a thread do plugin.
fn emit(out: &DapOut, outgoing: &Outgoing) {
    let body = match outgoing {
        Outgoing::Response(r) => serde_json::to_string(r),
        Outgoing::Event(e) => serde_json::to_string(e),
        _ => return,
    };
    if let Ok(s) = body {
        out.write_raw(&s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Sink `Write` que acumula tudo num buffer compartilhado, para inspecionar
    /// o que o `DapOut` produziu.
    #[derive(Clone)]
    struct SharedSink(Arc<Mutex<Vec<u8>>>);
    impl io::Write for SharedSink {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn pump_emits_one_output_event_per_line() {
        let sink = SharedSink(Arc::new(Mutex::new(Vec::new())));
        let out = DapOut::new(Box::new(sink.clone()), 0);

        // Três "linhas": uma normal, uma com byte não-UTF-8 (0xFF), e uma final
        // SEM `\n` (o servidor pode morrer no meio de uma linha).
        let input: Vec<u8> = b"alpha\n\xff\nbeta".to_vec();
        pump_stream(&input[..], "stdout", &out);

        let raw = String::from_utf8_lossy(&sink.0.lock().unwrap()).into_owned();
        // Cada linha vira um evento `output` na categoria certa.
        assert_eq!(raw.matches("\"event\":\"output\"").count(), 3);
        assert!(raw.contains("alpha\\n"));
        assert!(raw.contains("beta")); // última linha sem `\n` ainda é emitida
        assert!(raw.contains("\"category\":\"stdout\""));
        // O byte inválido não derruba nada (substituído por U+FFFD, emitido como
        // UTF-8 cru pelo serde, não escapado).
        assert!(raw.contains('\u{fffd}'));
    }
}
