//! Sessão DAP: estado e roteamento de requests. Independente de I/O — recebe um
//! [`Request`] e devolve as mensagens a enviar, o que a torna testável sem stream.
//!
//! Esta é a camada "fácil" (como o LSP): traduz DAP usando o `samp_sdk::debug` para
//! mapear linha ↔ endereço. A conexão com o plugin do servidor (Componente 2)
//! ainda não existe; por ora os breakpoints são só resolvidos a endereço.

use pawnpro_dbg_protocol::{Breakpoint, Command, Step};
use samp_sdk::debug::AmxDbg;
use serde_json::{Value, json};

use crate::messages::{Event, Request, Response};

/// Mensagem de saída do `session`. Mantém o `session` puro: ele decide, o
/// `main` executa o I/O (escreve no stdout ou fala com o plugin).
pub enum Outgoing {
    Response(Response),
    Event(Event),
    /// Subir o servidor do jogo como processo FILHO do adaptador. Morre junto com
    /// o adaptador (encerrar/reiniciar), sem o editor rastrear nada.
    SpawnServer(SpawnSpec),
    /// Conectar no plugin pelo id de sessão do socket local (pedido no `launch`).
    ConnectPlugin(String),
    /// Encaminhar um comando ao plugin (breakpoints/continue/step).
    ToPlugin(Command),
}

/// Comando do servidor a executar, mais as variáveis de depuração que o plugin lê.
pub struct SpawnSpec {
    pub exe: String,
    pub args: Vec<String>,
    pub cwd: String,
    pub session: String,
    pub amx_path: String,
    /// Locale do editor (ex.: `pt-BR`), para o plugin localizar as mensagens de
    /// erro. Vazio = inglês.
    pub locale: String,
}

/// Um breakpoint como pedido pelo editor (DAP), antes de resolver a linha em
/// endereço. Campos opcionais espelham os modificadores do DAP.
struct ReqBp {
    line: i32,
    condition: Option<String>,
    hit_condition: Option<String>,
    log_message: Option<String>,
}

#[derive(Default)]
pub struct Session {
    seq: i64,
    /// Bloco de debug do `.amx` em depuração (carregado no `launch`).
    dbg: Option<AmxDbg>,
    /// Breakpoints resolvidos: (linha-fonte, endereço de código).
    breakpoints: Vec<(i32, u32)>,
    /// Caminho do arquivo-fonte (o `source.path` que o editor enviou em
    /// `setBreakpoints`). Usado no `stackTrace` para o frame apontar à fonte —
    /// senão o editor mostra "Origem Desconhecida".
    source_path: Option<String>,
    terminated: bool,
}

impl Session {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn next_seq(&mut self) -> i64 {
        self.seq += 1;
        self.seq
    }

    /// Resposta `ok` simples a um request (o caso da maioria dos handlers).
    fn reply(&mut self, req: &Request, body: Value) -> Vec<Outgoing> {
        let seq = self.next_seq();
        vec![Outgoing::Response(Response::ok(seq, req, body))]
    }

    /// Resposta `ok` precedida de um comando ao plugin (continue/step/...). O
    /// comando vai antes da resposta para o plugin já receber a ação.
    fn reply_with(&mut self, req: &Request, cmd: Command, body: Value) -> Vec<Outgoing> {
        let seq = self.next_seq();
        vec![
            Outgoing::ToPlugin(cmd),
            Outgoing::Response(Response::ok(seq, req, body)),
        ]
    }

    #[must_use]
    pub fn is_terminated(&self) -> bool {
        self.terminated
    }

    /// Processa um request e devolve as mensagens a enviar (resposta + eventos).
    pub fn handle(&mut self, req: &Request) -> Vec<Outgoing> {
        match req.command.as_str() {
            "initialize" => self.on_initialize(req),
            "launch" => self.on_launch(req),
            "setBreakpoints" => self.on_set_breakpoints(req),
            "threads" => self.on_threads(req),
            "continue" => self.on_continue(req),
            "next" => self.on_step(req, Step::Over),
            "stepIn" => self.on_step(req, Step::In),
            "stepOut" => self.on_step(req, Step::Out),
            "stackTrace" => self.on_stack_trace(req),
            "scopes" => self.on_scopes(req),
            "variables" => self.on_variables(req),
            "setVariable" => self.on_set_variable(req),
            "evaluate" => self.on_evaluate(req),
            "disconnect" | "terminate" => self.on_disconnect(req),
            "restart" => self.on_restart(req),
            "configurationDone" => self.on_configuration_done(req),
            // Comandos ainda não implementados respondem ok vazio para não travar
            // o cliente.
            _ => self.ack(req),
        }
    }

    fn ack(&mut self, req: &Request) -> Vec<Outgoing> {
        self.reply(req, Value::Null)
    }

    fn on_initialize(&mut self, req: &Request) -> Vec<Outgoing> {
        // Capabilities mínimas da v1.
        let caps = json!({
            "supportsConfigurationDoneRequest": true,
            "supportsTerminateRequest": true,
            // Habilita avaliar variável ao passar o mouse no código (hover) — usa
            // o mesmo `evaluate` do painel INSPEÇÃO (watch).
            "supportsEvaluateForHovers": true,
            // Breakpoint condicional: o plugin avalia `var OP valor` e só pausa
            // se verdadeiro.
            "supportsConditionalBreakpoints": true,
            // Breakpoint por contagem de acertos (`5`, `>=3`, `%2`).
            "supportsHitConditionalBreakpoints": true,
            // Logpoint: breakpoint que loga `msg com {var}` sem pausar.
            "supportsLogPoints": true,
            // Editar variável no painel Variáveis durante a pausa.
            "supportsSetVariable": true,
            // NÃO declaramos `supportsRestartRequest`: assim o editor faz o
            // restart como disconnect + novo launch, que passa pelo nosso fluxo
            // (derruba o servidor antigo, espera a porta, sobe um novo) — o único
            // jeito de o código de carga rodar de novo.
        });
        let seq = self.next_seq();
        let resp = Response::ok(seq, req, caps);
        // DAP: após responder o initialize, emitir o evento `initialized`.
        let ev_seq = self.next_seq();
        let ev = Event::new(ev_seq, "initialized", Value::Null);
        vec![Outgoing::Response(resp), Outgoing::Event(ev)]
    }

    fn on_launch(&mut self, req: &Request) -> Vec<Outgoing> {
        // `arguments.program` = caminho do `.amx` compilado com `-d3`; dele
        // extraímos o bloco de debug (para mapear linha↔endereço).
        let amx_path = req
            .arguments
            .get("program")
            .or_else(|| req.arguments.get("debugInfo"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if let Ok(bytes) = std::fs::read(&amx_path)
            && let Ok(dbg) = AmxDbg::from_amx(&bytes).or_else(|_| AmxDbg::parse(&bytes))
        {
            self.dbg = Some(dbg);
        }
        // Origem-fonte padrão: o `.pwn` ao lado do `.amx`. Garante que o
        // `stackTrace` ancore o frame a um arquivo mesmo sem breakpoints (ex.: ao
        // pausar num erro de runtime). Um `setBreakpoints` posterior, se vier,
        // sobrescreve com o caminho exato que o editor conhece.
        if self.source_path.is_none() && amx_path.to_ascii_lowercase().ends_with(".amx") {
            self.source_path = Some(format!("{}.pwn", &amx_path[..amx_path.len() - 4]));
        }
        let session_id = req
            .arguments
            .get("session")
            .and_then(Value::as_str)
            .unwrap_or("default")
            .to_string();

        let mut out = Vec::new();

        // Sobe o servidor como processo filho, se a extensão passou o comando.
        // (Ele lê `PAWNPRO_DBG_SESSION`/`PAWNPRO_DBG_AMXDBG` do ambiente.)
        if let Some(cmd) = req.arguments.get("serverCommand") {
            let exe = cmd
                .get("exe")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if !exe.is_empty() {
                let args = cmd
                    .get("args")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                let cwd = cmd
                    .get("cwd")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let locale = req
                    .arguments
                    .get("locale")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                out.push(Outgoing::SpawnServer(SpawnSpec {
                    exe,
                    args,
                    cwd,
                    session: session_id.clone(),
                    amx_path,
                    locale,
                }));
            }
        }

        let seq = self.next_seq();
        // O adaptador conecta no plugin com retry (o servidor leva um tempo para
        // subir e abrir o socket), então a ordem spawn → connect não exige espera.
        out.push(Outgoing::ConnectPlugin(session_id));
        out.push(Outgoing::Response(Response::ok(seq, req, Value::Null)));
        out
    }

    fn on_set_breakpoints(&mut self, req: &Request) -> Vec<Outgoing> {
        let file = req
            .arguments
            .get("source")
            .and_then(|s| s.get("path"))
            .and_then(Value::as_str);
        // Guarda o caminho-fonte que o editor conhece, para o `stackTrace` poder
        // devolver um frame ancorado neste arquivo.
        if let Some(p) = file {
            self.source_path = Some(p.to_string());
        }
        // Cada breakpoint pode trazer `line`, `condition` (expressão), `hitCondition`
        // (contagem) e `logMessage` (logpoint). Todos opcionais; strings vazias
        // viram `None`.
        let str_opt = |b: &Value, key: &str| {
            b.get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        let requested: Vec<ReqBp> = req
            .arguments
            .get("breakpoints")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|b| {
                        let line = i32::try_from(b.get("line").and_then(Value::as_i64)?).ok()?;
                        // logMessage NÃO é trimado para `None` por espaços internos,
                        // mas vazio total vira None (não é logpoint).
                        let log_message = b
                            .get("logMessage")
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty())
                            .map(str::to_string);
                        Some(ReqBp {
                            line,
                            condition: str_opt(b, "condition"),
                            hit_condition: str_opt(b, "hitCondition"),
                            log_message,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        self.breakpoints.clear();
        let mut verified = Vec::new();
        let mut breakpoints = Vec::new();
        for ReqBp {
            line,
            condition,
            hit_condition,
            log_message,
        } in requested
        {
            let addr = self
                .dbg
                .as_ref()
                .and_then(|d| d.line_to_address(line, file));
            if let Some(a) = addr {
                self.breakpoints.push((line, a));
                breakpoints.push(Breakpoint {
                    addr: a,
                    condition,
                    hit_condition,
                    log_message,
                });
            }
            // Um breakpoint é "verificado" quando casou um endereço real. A linha
            // pedida pode não ser "quebrável" — o endereço desliza para a próxima
            // linha executável. Devolvemos a linha REAL (via `lookup_line` do
            // endereço) para o editor reposicionar o marcador onde a VM vai parar.
            let actual_line = addr
                .and_then(|a| self.dbg.as_ref().and_then(|d| d.lookup_line(a)))
                .unwrap_or(line);
            verified.push(json!({ "verified": addr.is_some(), "line": actual_line }));
        }

        let body = json!({ "breakpoints": verified });
        self.reply_with(req, Command::SetBreakpoints { breakpoints }, body)
    }

    /// `configurationDone`: o cliente terminou de enviar a configuração inicial
    /// (breakpoints). Sinaliza `Configured` ao plugin, que então libera a VM
    /// segura na carga — assim breakpoints em `OnGameModeInit` e afins são pegos.
    fn on_configuration_done(&mut self, req: &Request) -> Vec<Outgoing> {
        self.reply_with(req, Command::Configured, Value::Null)
    }

    /// `continue`: retoma a VM (manda `Continue` ao plugin).
    fn on_continue(&mut self, req: &Request) -> Vec<Outgoing> {
        self.reply_with(
            req,
            Command::Continue,
            json!({ "allThreadsContinued": true }),
        )
    }

    /// `next`/`stepIn`/`stepOut`: manda o step correspondente ao plugin.
    fn on_step(&mut self, req: &Request, mode: Step) -> Vec<Outgoing> {
        self.reply_with(req, Command::Step { mode }, Value::Null)
    }

    /// `stackTrace`: um único frame na linha onde a VM parou (v1 sem call stack
    /// completo — o plugin ainda não caminha os frames). O frame inclui `source`
    /// apontando ao arquivo-fonte; sem isso o editor mostra "Origem Desconhecida"
    /// e não destaca a linha de execução.
    fn on_stack_trace(&mut self, req: &Request) -> Vec<Outgoing> {
        let line = crate::plugin_client::last_line().unwrap_or(0);
        let mut frame = json!({
            "id": 1,
            "name": "main",
            "line": line,
            "column": 0,
        });
        if let Some(path) = self.source_path.as_deref() {
            frame["source"] = json!({
                "name": std::path::Path::new(path)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(path),
                "path": path,
            });
        }
        let body = json!({ "stackFrames": [frame], "totalFrames": 1 });
        self.reply(req, body)
    }

    /// `scopes`: um escopo "Locais" com `variablesReference` fixo (1).
    fn on_scopes(&mut self, req: &Request) -> Vec<Outgoing> {
        let body = json!({
            "scopes": [ { "name": "Locais", "variablesReference": 1, "expensive": false } ]
        });
        self.reply(req, body)
    }

    /// `variables`: devolve as variáveis da última pausa (recebidas no `Paused`).
    fn on_variables(&mut self, req: &Request) -> Vec<Outgoing> {
        let vars: Vec<Value> = crate::plugin_client::last_vars()
            .into_iter()
            .map(|v| json!({ "name": v.name, "value": v.value, "variablesReference": 0 }))
            .collect();
        let body = json!({ "variables": vars });
        self.reply(req, body)
    }

    /// `setVariable`: edita uma variável no painel Variáveis durante a pausa. O
    /// novo valor (`value`) é um inteiro (decimal ou `0x` hex). Encaminha ao
    /// plugin, que escreve na célula da VM, e responde ao editor com o valor.
    fn on_set_variable(&mut self, req: &Request) -> Vec<Outgoing> {
        let name = req
            .arguments
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let raw = req
            .arguments
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();

        // Aceita inteiro (decimal/hex), float (`50.0`) e bool (`true`/`false`). O
        // valor enviado ao plugin é sempre uma célula i32 (float = bits IEEE-754,
        // bool = 0/1); `shown` é o texto amigável que volta para o painel.
        let seq = self.next_seq();
        let Some((value, shown)) = parse_set_value(&raw) else {
            return vec![Outgoing::Response(Response::fail(
                seq,
                req,
                format!(
                    "valor inválido: '{raw}' (use inteiro, ex.: 100/0x64; float, ex.: 1.5; ou true/false)"
                ),
            ))];
        };

        // Arrays não são editáveis (o plugin os rejeita). Detectamos pelo valor
        // atual em cache começar com `[` e falhamos AQUI, em vez de responder um
        // sucesso falso e desencontrar o painel do estado real da VM.
        let is_array = crate::plugin_client::last_vars()
            .iter()
            .any(|v| v.name == name && v.value.trim_start().starts_with('['));
        if is_array {
            return vec![Outgoing::Response(Response::fail(
                seq,
                req,
                format!("'{name}' é um array; editar arrays ainda não é suportado"),
            ))];
        }

        // Resposta otimista: a edição quase sempre vale (variável simples em
        // escopo). O plugin efetiva a escrita; atualizamos o cache local para o
        // painel/watch refletirem o novo valor sem reler a VM.
        crate::plugin_client::update_var(&name, &shown);
        let body = json!({ "value": shown, "variablesReference": 0 });
        vec![
            Outgoing::ToPlugin(Command::SetVariable { name, value }),
            Outgoing::Response(Response::ok(seq, req, body)),
        ]
    }

    /// `evaluate`: usado pelo painel INSPEÇÃO (watch) e pelo hover. Avalia uma
    /// expressão simples — por ora, o NOME de uma variável em escopo — buscando
    /// nas variáveis da última pausa. Expressões compostas ainda não são
    /// suportadas; nesses casos respondemos com erro amigável (DAP exige falha no
    /// `evaluate` para o editor mostrar "não disponível" em vez de um valor falso).
    fn on_evaluate(&mut self, req: &Request) -> Vec<Outgoing> {
        let expr = req
            .arguments
            .get("expression")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();

        // Busca exata pelo nome da variável entre as da última pausa.
        let found = crate::plugin_client::last_vars()
            .into_iter()
            .find(|v| v.name == expr);

        let seq = self.next_seq();
        if let Some(v) = found {
            let body = json!({ "result": v.value, "variablesReference": 0 });
            vec![Outgoing::Response(Response::ok(seq, req, body))]
        } else {
            // Sem a variável em escopo (ou expressão composta): falha explícita.
            let detail = if expr.is_empty() {
                "expressão vazia".to_string()
            } else {
                format!("'{expr}' não está em escopo")
            };
            vec![Outgoing::Response(Response::fail(seq, req, detail))]
        }
    }

    fn on_threads(&mut self, req: &Request) -> Vec<Outgoing> {
        // Uma única "thread" lógica — o servidor Pawn roda um VM.
        let body = json!({ "threads": [ { "id": 1, "name": "main" } ] });
        self.reply(req, body)
    }

    fn on_disconnect(&mut self, req: &Request) -> Vec<Outgoing> {
        self.terminated = true;
        let seq = self.next_seq();
        let resp = Response::ok(seq, req, Value::Null);
        let ev_seq = self.next_seq();
        let ev = Event::new(ev_seq, "terminated", Value::Null);
        vec![Outgoing::Response(resp), Outgoing::Event(ev)]
    }

    /// `restart`: reiniciar exige um ciclo COMPLETO — recompilar e subir um
    /// servidor novo (senão o código de carga não roda de novo). Em vez
    /// de um restart "leve" (que só relança o adaptador e reconecta no servidor
    /// velho), encerramos a sessão com `terminated` + `restart`, fazendo o editor
    /// relançar do zero — passando pelo nosso fluxo que derruba o servidor antigo,
    /// espera a porta e sobe um novo.
    fn on_restart(&mut self, req: &Request) -> Vec<Outgoing> {
        self.terminated = true;
        let seq = self.next_seq();
        let resp = Response::ok(seq, req, Value::Null);
        let ev_seq = self.next_seq();
        let ev = Event::new(ev_seq, "terminated", json!({ "restart": true }));
        vec![Outgoing::Response(resp), Outgoing::Event(ev)]
    }

    /// Breakpoints resolvidos a endereço (para o plugin, no futuro).
    #[allow(dead_code)] // consumido pelo Componente 2 (envio ao plugin)
    #[must_use]
    pub fn resolved_breakpoints(&self) -> &[(i32, u32)] {
        &self.breakpoints
    }

    /// Injeta um bloco de debug diretamente (usado em testes e quando a extração
    /// do `.amx` é feita por fora).
    #[allow(dead_code)] // usado em testes e pela integração futura
    pub fn set_debug(&mut self, dbg: AmxDbg) {
        self.dbg = Some(dbg);
    }
}

/// Interpreta o texto digitado em `setVariable` e devolve `(célula, texto)`:
/// - a **célula** é o `i32` gravado na VM (float → bits IEEE-754; bool → 0/1);
/// - o **texto** é a forma amigável que volta ao painel (`50`, `1.5`, `true`).
///
/// Aceita: `true`/`false`, hex `0x..`, inteiro decimal, e float (`1.5`). `None`
/// se nada casar.
fn parse_set_value(raw: &str) -> Option<(i32, String)> {
    match raw {
        "true" => return Some((1, "true".to_string())),
        "false" => return Some((0, "false".to_string())),
        _ => {}
    }
    if let Some(hex) = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X"))
        && let Ok(i) = i32::from_str_radix(hex, 16)
    {
        return Some((i, i.to_string()));
    }
    if let Ok(i) = raw.parse::<i32>() {
        return Some((i, i.to_string()));
    }
    if let Ok(f) = raw.parse::<f32>() {
        // Grava os bits do float; mostra o float (não os bits). Mantém ao menos
        // uma casa decimal para o painel não exibir um float como se fosse int
        // (ex.: `50.0` viraria `50` com a formatação padrão).
        let shown = if f.fract() == 0.0 {
            format!("{f:.1}")
        } else {
            format!("{f}")
        };
        return Some((f.to_bits().cast_signed(), shown));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_set_value_types() {
        // Inteiro decimal/hex.
        assert_eq!(parse_set_value("100"), Some((100, "100".to_string())));
        assert_eq!(parse_set_value("-5"), Some((-5, "-5".to_string())));
        assert_eq!(parse_set_value("0x64"), Some((100, "100".to_string())));
        // Bool.
        assert_eq!(parse_set_value("true"), Some((1, "true".to_string())));
        assert_eq!(parse_set_value("false"), Some((0, "false".to_string())));
        // Float: célula = bits IEEE-754; texto preserva o `.0`.
        let (cell, shown) = parse_set_value("50.0").unwrap();
        assert_eq!(cell, 50.0f32.to_bits().cast_signed());
        assert_eq!(shown, "50.0");
        let (cell, shown) = parse_set_value("1.5").unwrap();
        assert_eq!(cell, 1.5f32.to_bits().cast_signed());
        assert_eq!(shown, "1.5");
        // Inválido.
        assert_eq!(parse_set_value("abc"), None);
        assert_eq!(parse_set_value(""), None);
    }

    fn req(command: &str, args: &Value) -> Request {
        // Request não é Deserialize-friendly de construir à mão; via JSON.
        serde_json::from_value(json!({
            "seq": 1, "type": "request", "command": command, "arguments": args
        }))
        .unwrap()
    }

    /// Primeira Response na lista de saída.
    fn first_response(out: &[Outgoing]) -> &Response {
        out.iter()
            .find_map(|o| match o {
                Outgoing::Response(r) => Some(r),
                _ => None,
            })
            .expect("esperava uma Response")
    }
    /// `true` se há um Event com o nome dado.
    fn has_event(out: &[Outgoing], name: &str) -> bool {
        out.iter()
            .any(|o| matches!(o, Outgoing::Event(e) if e.event == name))
    }
    /// `true` se há um comando para o plugin que casa o predicado.
    fn has_command(out: &[Outgoing], pred: impl Fn(&Command) -> bool) -> bool {
        out.iter()
            .any(|o| matches!(o, Outgoing::ToPlugin(c) if pred(c)))
    }

    #[test]
    fn initialize_emits_capabilities_and_initialized() {
        let mut s = Session::new();
        let out = s.handle(&req("initialize", &Value::Null));
        let r = first_response(&out);
        assert!(r.success && r.command == "initialize");
        assert!(has_event(&out, "initialized"));
    }

    #[test]
    fn threads_returns_single_thread() {
        let mut s = Session::new();
        let out = s.handle(&req("threads", &Value::Null));
        assert_eq!(first_response(&out).body["threads"][0]["id"], 1);
    }

    #[test]
    fn set_breakpoints_resolves_and_forwards_addrs() {
        let mut s = Session::new();
        s.set_debug(sample_dbg());
        // Linhas são 1-based (a lib soma +1 ao zero-based do compilador): a
        // entrada gravada como `3` é a linha 4 para o editor, no endereço 20.
        let args = json!({
            "source": { "path": "a.pwn" },
            "breakpoints": [ { "line": 4 }, { "line": 999 } ]
        });
        let out = s.handle(&req("setBreakpoints", &args));
        let bps = first_response(&out).body["breakpoints"].as_array().unwrap();
        assert_eq!(bps[0]["verified"], true); // linha 4 existe
        assert_eq!(bps[1]["verified"], false); // linha 999 não
        assert_eq!(s.resolved_breakpoints(), &[(4, 20)]);
        // O endereço resolvido é encaminhado ao plugin (sem condição).
        assert!(has_command(
            &out,
            |c| matches!(c, Command::SetBreakpoints { breakpoints }
            if breakpoints.len() == 1 && breakpoints[0].addr == 20 && breakpoints[0].condition.is_none())
        ));
    }

    #[test]
    fn set_breakpoints_forwards_condition() {
        let mut s = Session::new();
        s.set_debug(sample_dbg());
        let args = json!({
            "source": { "path": "a.pwn" },
            "breakpoints": [ { "line": 4, "condition": "x == 5" } ]
        });
        let out = s.handle(&req("setBreakpoints", &args));
        assert!(has_command(
            &out,
            |c| matches!(c, Command::SetBreakpoints { breakpoints }
            if breakpoints.len() == 1
                && breakpoints[0].addr == 20
                && breakpoints[0].condition.as_deref() == Some("x == 5"))
        ));
    }

    #[test]
    fn set_breakpoints_forwards_hit_condition_and_logpoint() {
        let mut s = Session::new();
        s.set_debug(sample_dbg());
        let args = json!({
            "source": { "path": "a.pwn" },
            "breakpoints": [
                { "line": 4, "hitCondition": ">=3", "logMessage": "x={x}" }
            ]
        });
        let out = s.handle(&req("setBreakpoints", &args));
        assert!(has_command(
            &out,
            |c| matches!(c, Command::SetBreakpoints { breakpoints }
            if breakpoints.len() == 1
                && breakpoints[0].hit_condition.as_deref() == Some(">=3")
                && breakpoints[0].log_message.as_deref() == Some("x={x}"))
        ));
    }

    #[test]
    fn initialize_advertises_logpoints_and_hit_count() {
        let mut s = Session::new();
        let out = s.handle(&req("initialize", &Value::Null));
        let caps = &first_response(&out).body;
        assert_eq!(caps["supportsLogPoints"], true);
        assert_eq!(caps["supportsHitConditionalBreakpoints"], true);
    }

    #[test]
    fn continue_and_steps_forward_to_plugin() {
        let mut s = Session::new();
        assert!(has_command(
            &s.handle(&req("continue", &Value::Null)),
            |c| matches!(c, Command::Continue)
        ));
        assert!(has_command(
            &s.handle(&req("next", &Value::Null)),
            |c| matches!(c, Command::Step { mode: Step::Over })
        ));
        assert!(has_command(
            &s.handle(&req("stepIn", &Value::Null)),
            |c| matches!(c, Command::Step { mode: Step::In })
        ));
        assert!(has_command(
            &s.handle(&req("stepOut", &Value::Null)),
            |c| matches!(c, Command::Step { mode: Step::Out })
        ));
    }

    #[test]
    fn launch_requests_plugin_connection() {
        let mut s = Session::new();
        let out = s.handle(&req("launch", &json!({ "session": "abc" })));
        assert!(
            out.iter()
                .any(|o| matches!(o, Outgoing::ConnectPlugin(id) if id == "abc"))
        );
    }

    #[test]
    fn launch_derives_source_from_program_for_stacktrace() {
        // Sem setBreakpoints, o stackTrace deve ancorar o frame ao `.pwn` derivado
        // do `program` (.amx) — senão o editor mostra "Origem Desconhecida" ao
        // pausar num erro de runtime.
        let mut s = Session::new();
        s.set_debug(sample_dbg());
        s.handle(&req("launch", &json!({ "program": "/srv/gm/molde.amx" })));
        let out = s.handle(&req("stackTrace", &Value::Null));
        let frame = &first_response(&out).body["stackFrames"][0];
        assert_eq!(frame["source"]["path"], "/srv/gm/molde.pwn");
    }

    #[test]
    fn disconnect_terminates() {
        let mut s = Session::new();
        let out = s.handle(&req("disconnect", &Value::Null));
        assert!(s.is_terminated());
        assert!(has_event(&out, "terminated"));
    }

    /// Bloco de debug mínimo (mesma forma do teste do amxdbg): a.pwn linha 3 → 20.
    fn sample_dbg() -> AmxDbg {
        let mut t = Vec::new();
        // files: 1 (a.pwn @ 0)
        ext_u32(&mut t, 0);
        ext_cstr(&mut t, "a.pwn");
        // lines: 2 — (8,2), (20,3)
        ext_u32(&mut t, 8);
        ext_i32(&mut t, 2);
        ext_u32(&mut t, 20);
        ext_i32(&mut t, 3);
        let mut b = Vec::new();
        ext_i32(&mut b, i32::try_from(22 + t.len()).unwrap());
        b.extend_from_slice(&samp_sdk::debug::AMX_DBG_MAGIC.to_le_bytes());
        b.push(1);
        b.push(1);
        ext_i16(&mut b, 0); // flags
        ext_i16(&mut b, 1); // files
        ext_i16(&mut b, 2); // lines
        ext_i16(&mut b, 0); // symbols
        ext_i16(&mut b, 0); // tags
        ext_i16(&mut b, 0); // automatons
        ext_i16(&mut b, 0); // states
        b.extend_from_slice(&t);
        AmxDbg::parse(&b).unwrap()
    }

    fn ext_i16(v: &mut Vec<u8>, x: i16) {
        v.extend_from_slice(&x.to_le_bytes());
    }
    fn ext_u32(v: &mut Vec<u8>, x: u32) {
        v.extend_from_slice(&x.to_le_bytes());
    }
    fn ext_i32(v: &mut Vec<u8>, x: i32) {
        v.extend_from_slice(&x.to_le_bytes());
    }
    fn ext_cstr(v: &mut Vec<u8>, s: &str) {
        v.extend_from_slice(s.as_bytes());
        v.push(0);
    }
}
