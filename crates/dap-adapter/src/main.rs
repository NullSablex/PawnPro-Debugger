//! Adaptador DAP (Componente 3 do debugger) — processo lançado pelo editor.
//! Traduz Debug Adapter Protocol (DAP) ⇄ protocolo do plugin, usando
//! `samp_sdk::debug` para mapear linha ↔ endereço.
//!
//! Loop síncrono sobre stdin/stdout (igual a um LSP básico). Uma thread separada
//! recebe eventos do plugin (socket local) e os escreve como eventos DAP no stdout.

mod expr;
mod messages;
mod plugin_client;
mod protocol;
mod session;

use std::io::{self, BufReader};
use std::process::Child;
use std::sync::Arc;

use messages::{Request, Response};
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
                // Subir e reiniciar são o mesmo ato: derrubar o que houver e
                // pôr um servidor no lugar. No `launch` não há o que derrubar;
                // no `restart` há, e o `Drop` do `ServerChild` cuida disso ao
                // atribuir `None`. Reconectar depois é inofensivo no primeiro
                // caso — o `launch` emite `ConnectPlugin` logo em seguida — e
                // necessário no segundo, porque o canal morreu com o processo.
                Outgoing::SpawnServer(spec) => {
                    server = None;
                    match spawn_server(&spec, &out) {
                        Ok(child) => {
                            server = Some(child);
                            // Só conecta se ainda não há cliente. Trocar o
                            // `PluginClient` jogaria fora a fila de comandos que
                            // ele acumulou — e o editor manda `setBreakpoints`
                            // ANTES do `launch`, então é justamente ali que os
                            // breakpoints estão esperando.
                            if plugin.is_none() {
                                plugin = Some(PluginClient::connect(&spec.session, out.clone()));
                            }
                        }
                        Err(e) => out.event(
                            "output",
                            serde_json::json!({
                                "category": "stderr",
                                "output": format!("Falha ao iniciar o servidor: {e}\n"),
                            }),
                        ),
                    }
                }
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
                Outgoing::ReadMemory {
                    seq,
                    address,
                    frame,
                    name,
                    index,
                    offset,
                    count,
                } => {
                    // Leitura bloqueante no plugin (com timeout), então a resposta.
                    let bytes = plugin
                        .as_ref()
                        .and_then(|c| c.read_memory(frame, name, index, offset, count))
                        .unwrap_or_default();
                    let body = serde_json::json!({
                        "address": address,
                        "data": base64_encode(&bytes),
                    });
                    emit(&out, &Outgoing::Response(Response::ok(seq, &req, body)));
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

/// Bytes do console do servidor em texto.
///
/// O SA-MP/open.mp escreve o console em Windows-1252, não em UTF-8: em
/// português `ção` sai como `\xe7\xe3o`, que `from_utf8_lossy` trocaria por `�`.
/// Tentamos UTF-8 primeiro — é o que um gamemode moderno pode emitir — e só
/// caímos no cp1252 quando a sequência não é UTF-8 válida.
///
/// A conversão é direta e sem dependência: em cp1252 os bytes 0xA0–0xFF já são
/// os mesmos code points Unicode (herança do Latin-1); apenas 0x80–0x9F têm
/// tabela própria.
fn decodificar_console(bytes: &[u8]) -> String {
    /// Os 32 code points de 0x80–0x9F, onde cp1252 difere do Latin-1.
    const ALTOS: [char; 32] = [
        '\u{20AC}', '\u{81}', '\u{201A}', '\u{192}', '\u{201E}', '\u{2026}', '\u{2020}',
        '\u{2021}', '\u{2C6}', '\u{2030}', '\u{160}', '\u{2039}', '\u{152}', '\u{8D}', '\u{17D}',
        '\u{8F}', '\u{90}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}',
        '\u{2014}', '\u{2DC}', '\u{2122}', '\u{161}', '\u{203A}', '\u{153}', '\u{9D}', '\u{17E}',
        '\u{178}',
    ];
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_owned();
    }
    bytes
        .iter()
        .map(|&b| match b {
            0x80..=0x9F => ALTOS[usize::from(b - 0x80)],
            _ => char::from(b),
        })
        .collect()
}

/// Lógica síncrona de `forward_stream`, isolada para ser testável sem thread.
/// Lê `stream` linha a linha e emite cada linha como `output`. `read_until('\n')`
/// (em vez de `lines()`) preserva a quebra de linha e a última linha sem `\n`.
fn pump_stream<R: io::Read>(stream: R, category: &'static str, out: &DapOut) {
    use std::io::BufRead;
    let mut reader = BufReader::new(stream);
    let mut buf = Vec::new();
    while let Ok(n) = reader.read_until(b'\n', &mut buf) {
        if n == 0 {
            break; // EOF — o servidor fechou a pipe
        }
        let text = decodificar_console(&buf);
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

/// Codifica bytes em base64 (alfabeto padrão) — o campo `data` do `readMemory`
/// do DAP é base64. Evita uma dependência externa para algo tão pequeno.
fn base64_encode(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b1 = u32::from(chunk[0]);
        let b2 = u32::from(chunk.get(1).copied().unwrap_or(0));
        let b3 = u32::from(chunk.get(2).copied().unwrap_or(0));
        let n = (b1 << 16) | (b2 << 8) | b3;
        out.push(A[(n >> 18 & 63) as usize] as char);
        out.push(A[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            A[(n >> 6 & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            A[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
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

    /// O console do SA-MP/open.mp é Windows-1252: `from_utf8_lossy` trocava
    /// cada acento por `\u{FFFD}` no CONSOLE DE DEPURAÇÃO.
    #[test]
    fn console_decodifica_windows1252() {
        // Bytes como o servidor os escreve: "ção" = 0xE7 0xE3 0x6F.
        let cru = b"deslocamento=4 (acentua\xe7\xe3o: cora\xe7\xe3o)\n";
        assert_eq!(
            decodificar_console(cru),
            "deslocamento=4 (acentuação: coração)\n"
        );
    }

    /// UTF-8 válido tem prioridade: um gamemode moderno pode emitir UTF-8, e
    /// interpretá-lo como cp1252 daria mojibake ao contrário.
    #[test]
    fn console_preserva_utf8_valido() {
        assert_eq!(decodificar_console("ação".as_bytes()), "ação");
        assert_eq!(decodificar_console(b"plain ascii"), "plain ascii");
    }

    #[test]
    fn base64_encode_matches_rfc() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"M"), "TQ==");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"hello"), "aGVsbG8=");
    }

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
        // Byte que não é UTF-8 válido não derruba nada e não vira `\u{FFFD}`:
        // cai na leitura cp1252, onde 0xFF é `ÿ`.
        assert!(raw.contains('ÿ'));
        assert!(!raw.contains('\u{fffd}'));
    }
}
