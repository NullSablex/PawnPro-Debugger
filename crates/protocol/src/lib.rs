//! Protocolo próprio entre o plugin (no servidor) e o adaptador DAP — JSON por
//! linha (NDJSON) sobre um socket local (Unix domain socket no Linux/macOS,
//! named pipe no Windows; ver [`transport`]). Simples de propósito: o adaptador
//! é quem fala DAP com o editor; aqui só trafegam comandos e eventos crus.
//!
//! Crate compartilhado para que plugin (`debug-plugin`) e adaptador
//! (`dap-adapter`) usem exatamente os mesmos tipos.
//!
//! Direções:
//! - **Adaptador → plugin**: [`Command`] (breakpoints, continue, step).
//! - **Plugin → adaptador**: [`Event`] (parou com variáveis, saiu).

use serde::{Deserialize, Serialize};

pub mod transport;

/// Modo de step pedido pelo adaptador.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Step {
    In,
    Over,
    Out,
}

/// Comando do adaptador para o plugin.
/// Um breakpoint resolvido: endereço de código, mais modificadores opcionais.
///
/// - `condition`: expressão `var OP valor`, avaliada no plugin; só dispara se
///   verdadeira.
/// - `hit_condition`: condição sobre a CONTAGEM de acertos (`5`, `>=3`, `%2`);
///   dispara só quando o nº de vezes que o endereço foi atingido a satisfaz.
/// - `log_message`: se presente, o breakpoint é um **logpoint** — em vez de
///   pausar, o plugin interpola a mensagem (trechos `{expr}` viram o valor da
///   variável) e a emite como saída, seguindo a execução.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Breakpoint {
    pub addr: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hit_condition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "camelCase")]
pub enum Command {
    /// Substitui o conjunto de breakpoints (endereço + condição opcional).
    SetBreakpoints { breakpoints: Vec<Breakpoint> },
    /// Retoma a execução livre.
    Continue,
    /// Executa um passo no modo dado.
    Step { mode: Step },
    /// O adaptador terminou a configuração inicial (já enviou os breakpoints).
    /// Libera a VM que ficou segura na carga, para não perder breakpoints em
    /// código que roda uma única vez no início (ex.: `OnGameModeInit`).
    Configured,
    /// Edita uma variável em escopo na pausa atual: grava `value` na célula de
    /// `name`. Só vale enquanto a VM está pausada.
    SetVariable { name: String, value: i32 },
}

/// Evento do plugin para o adaptador.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "camelCase")]
pub enum Event {
    /// A VM pausou. `line` é a linha-fonte (mapeada do `cip`, se possível) e
    /// `vars` são os símbolos em escopo no momento — enviados junto para evitar
    /// uma ida-e-volta de inspeção enquanto a VM está bloqueada.
    Paused {
        reason: String,
        line: Option<i32>,
        vars: Vec<Var>,
        /// Texto descritivo opcional (ex.: mensagem de um erro de runtime quando
        /// `reason == "exception"`). Vira o `description`/`text` do `stopped` DAP.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
    /// Saída de um **logpoint**: a mensagem já interpolada. A VM NÃO pausa; o
    /// adaptador encaminha como um evento `output` do DAP (canal do console).
    Output { text: String },
    /// O script terminou / a VM foi descarregada.
    Exited,
}

/// Um par variável→valor para a inspeção.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Var {
    pub name: String,
    pub value: String,
}

/// Serializa uma mensagem como uma linha JSON (com `\n` ao final).
///
/// # Errors
/// Erro de serialização do serde (não deve ocorrer para estes tipos).
pub fn to_line<T: Serialize>(msg: &T) -> serde_json::Result<String> {
    let mut s = serde_json::to_string(msg)?;
    s.push('\n');
    Ok(s)
}

/// Desserializa uma linha JSON numa mensagem.
///
/// # Errors
/// Erro de desserialização (linha malformada ou variante desconhecida).
pub fn from_line<T: for<'de> Deserialize<'de>>(line: &str) -> serde_json::Result<T> {
    serde_json::from_str(line.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_roundtrip() {
        for cmd in [
            Command::SetBreakpoints {
                breakpoints: vec![
                    Breakpoint {
                        addr: 20,
                        condition: None,
                        hit_condition: None,
                        log_message: None,
                    },
                    Breakpoint {
                        addr: 48,
                        condition: Some("x == 5".into()),
                        hit_condition: Some(">=3".into()),
                        log_message: Some("x={x}".into()),
                    },
                ],
            },
            Command::Continue,
            Command::Step { mode: Step::Over },
        ] {
            let line = to_line(&cmd).unwrap();
            assert!(line.ends_with('\n'));
            let back: Command = from_line(&line).unwrap();
            assert_eq!(cmd, back);
        }
    }

    #[test]
    fn event_roundtrip() {
        for ev in [
            Event::Paused {
                reason: "breakpoint".into(),
                line: Some(42),
                vars: vec![Var {
                    name: "g".into(),
                    value: "1".into(),
                }],
                description: None,
            },
            Event::Paused {
                reason: "exception".into(),
                line: Some(7),
                vars: vec![],
                description: Some("divisão por zero".into()),
            },
            Event::Output { text: "x=5".into() },
            Event::Exited,
        ] {
            let line = to_line(&ev).unwrap();
            let back: Event = from_line(&line).unwrap();
            assert_eq!(ev, back);
        }
    }

    #[test]
    fn command_tag_is_stable() {
        // O adaptador depende destes nomes — fixa o formato do protocolo.
        let line = to_line(&Command::Step { mode: Step::In }).unwrap();
        assert!(line.contains("\"cmd\":\"step\""));
        assert!(line.contains("\"mode\":\"in\""));
    }
}
