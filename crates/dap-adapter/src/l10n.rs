//! Localização das mensagens do adaptador voltadas ao editor. Espelha os 5
//! idiomas dos erros de runtime do plugin (pt-BR, en, es, ru, ro). O locale vem
//! do argumento `locale` do `initialize` (o cliente informa o idioma).

/// Idioma das mensagens. Mesma resolução por prefixo do plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Locale {
    PtBr,
    Es,
    Ru,
    Ro,
    #[default]
    En,
}

impl Locale {
    /// Resolve do código de locale (`pt-BR`, `es`, …). Desconhecido → inglês.
    #[must_use]
    pub fn from_code(s: &str) -> Self {
        let s = s.to_ascii_lowercase();
        if s.starts_with("pt") {
            Self::PtBr
        } else if s.starts_with("es") {
            Self::Es
        } else if s.starts_with("ru") {
            Self::Ru
        } else if s.starts_with("ro") {
            Self::Ro
        } else {
            Self::En
        }
    }
}

/// Mensagem localizável do adaptador (com seus argumentos).
pub enum Msg<'a> {
    /// Rótulo do filtro de exceção "Erros de runtime".
    RuntimeErrorsLabel,
    /// Valor inválido em `setVariable` (`{raw}` = texto digitado).
    InvalidValue(&'a str),
    /// Elemento de array inválido (`{name}` = nome do filho, ex.: `[i]`).
    InvalidElement(&'a str),
    /// Tentativa de editar o array inteiro (`{name}` = nome do array).
    ArrayEditElement(&'a str),
    /// Expressão vazia no `evaluate`.
    EmptyExpression,
    /// Falha ao avaliar uma expressão (`{expr}`).
    CannotEvaluate(&'a str),
}

impl Msg<'_> {
    /// Texto no idioma dado.
    #[must_use]
    pub fn text(&self, locale: Locale) -> String {
        use Locale::{En, Es, PtBr, Ro, Ru};
        use Msg::{
            ArrayEditElement, CannotEvaluate, EmptyExpression, InvalidElement, InvalidValue,
            RuntimeErrorsLabel,
        };
        match (self, locale) {
            (RuntimeErrorsLabel, PtBr) => "Erros de runtime".into(),
            (RuntimeErrorsLabel, Es) => "Errores de runtime".into(),
            (RuntimeErrorsLabel, Ru) => "Ошибки времени выполнения".into(),
            (RuntimeErrorsLabel, Ro) => "Erori de runtime".into(),
            (RuntimeErrorsLabel, En) => "Runtime errors".into(),

            (InvalidValue(r), PtBr) => format!(
                "valor inválido: '{r}' (use inteiro, ex.: 100/0x64; float, ex.: 1.5; ou true/false)"
            ),
            (InvalidValue(r), Es) => format!(
                "valor inválido: '{r}' (use un entero, ej.: 100/0x64; un float, ej.: 1.5; o true/false)"
            ),
            (InvalidValue(r), Ru) => format!(
                "недопустимое значение: '{r}' (целое, напр. 100/0x64; дробное, напр. 1.5; или true/false)"
            ),
            (InvalidValue(r), Ro) => format!(
                "valoare invalidă: '{r}' (folosiți un întreg, ex.: 100/0x64; un float, ex.: 1.5; sau true/false)"
            ),
            (InvalidValue(r), En) => format!(
                "invalid value: '{r}' (use an integer, e.g. 100/0x64; a float, e.g. 1.5; or true/false)"
            ),

            (InvalidElement(n), PtBr | Es) => format!("elemento inválido: '{n}'"),
            (InvalidElement(n), Ru) => format!("недопустимый элемент: '{n}'"),
            (InvalidElement(n), Ro) => format!("element invalid: '{n}'"),
            (InvalidElement(n), En) => format!("invalid element: '{n}'"),

            (ArrayEditElement(n), PtBr) => {
                format!("'{n}' é um array; expanda e edite um elemento (ex.: {n}[0])")
            }
            (ArrayEditElement(n), Es) => {
                format!("'{n}' es un array; expándalo y edite un elemento (ej.: {n}[0])")
            }
            (ArrayEditElement(n), Ru) => {
                format!("'{n}' — массив; разверните его и измените элемент (напр. {n}[0])")
            }
            (ArrayEditElement(n), Ro) => {
                format!("'{n}' este un array; extindeți-l și editați un element (ex.: {n}[0])")
            }
            (ArrayEditElement(n), En) => {
                format!("'{n}' is an array; expand it and edit an element (e.g. {n}[0])")
            }

            (EmptyExpression, PtBr) => "expressão vazia".into(),
            (EmptyExpression, Es) => "expresión vacía".into(),
            (EmptyExpression, Ru) => "пустое выражение".into(),
            (EmptyExpression, Ro) => "expresie goală".into(),
            (EmptyExpression, En) => "empty expression".into(),

            (CannotEvaluate(e), PtBr) => format!("não foi possível avaliar '{e}'"),
            (CannotEvaluate(e), Es) => format!("no se pudo evaluar '{e}'"),
            (CannotEvaluate(e), Ru) => format!("не удалось вычислить '{e}'"),
            (CannotEvaluate(e), Ro) => format!("nu s-a putut evalua '{e}'"),
            (CannotEvaluate(e), En) => format!("could not evaluate '{e}'"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locale_from_code() {
        assert_eq!(Locale::from_code("pt-BR"), Locale::PtBr);
        assert_eq!(Locale::from_code("ES"), Locale::Es);
        assert_eq!(Locale::from_code("ru-RU"), Locale::Ru);
        assert_eq!(Locale::from_code("ro"), Locale::Ro);
        assert_eq!(Locale::from_code("en-US"), Locale::En);
        assert_eq!(Locale::from_code("zh"), Locale::En);
        assert_eq!(Locale::default(), Locale::En);
    }

    #[test]
    fn messages_localized() {
        assert_eq!(Msg::EmptyExpression.text(Locale::PtBr), "expressão vazia");
        assert_eq!(Msg::EmptyExpression.text(Locale::En), "empty expression");
        assert_eq!(
            Msg::CannotEvaluate("x+").text(Locale::En),
            "could not evaluate 'x+'"
        );
        assert_eq!(
            Msg::ArrayEditElement("arr").text(Locale::En),
            "'arr' is an array; expand it and edit an element (e.g. arr[0])"
        );
        assert_eq!(
            Msg::RuntimeErrorsLabel.text(Locale::Ru),
            "Ошибки времени выполнения"
        );
    }
}
