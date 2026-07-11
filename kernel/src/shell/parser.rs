//! Parser de línea de comandos de la shell.
// COMPILE-STATUS: borrador (sin compilar)
//!
//! Tokenizador REAL que respeta comillas simples (`'…'`) y dobles (`"…"`),
//! escapes con barra invertida (`\`), y detecta los operadores de la shell:
//! redirección de salida `>` (truncar) y `>>` (añadir), y tubería `|`.
//!
//! El resultado es un [`Command`] OWNED (usa `String`/`Vec` para sobrevivir a
//! la línea de entrada) más su información de redirección. Para tuberías se
//! ofrece [`parse_pipeline`], que devuelve una etapa [`Command`] por cada
//! sección separada por `|`; [`parse`] (la firma canónica del contrato)
//! devuelve la primera etapa.
//!
//! El tokenizador es puro y determinista (sólo depende del `&str` de entrada),
//! por lo que es directamente testeable (ver `mod tests`).
#![allow(dead_code)]

use crate::prelude::*;

/// Redirección de salida detectada por el parser. [CANÓNICO]
///
/// Los `String` contienen el nombre del archivo destino ya des-entrecomillado.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Redirect {
    /// Sin redirección: la salida va a la consola.
    None,
    /// `> archivo`: trunca el archivo y escribe desde el principio.
    Truncate(String),
    /// `>> archivo`: crea si no existe y añade al final.
    Append(String),
}

/// Comando parseado (OWNED, para sobrevivir a la línea de entrada). [CANÓNICO]
///
/// `name` es el ejecutable/comando interno; `args` son SOLO los argumentos
/// (no incluye `name`); `redirect` describe la redirección de su salida.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Command {
    pub name: String,
    pub args: Vec<String>,
    pub redirect: Redirect,
}

/// Unidad léxica producida por el tokenizador.
///
/// Público para poder verificar el tokenizador de forma aislada.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Token {
    /// Una palabra (comando, argumento o nombre de archivo) ya sin comillas.
    Word(String),
    /// Operador `>` (redirección truncando).
    RedirectOut,
    /// Operador `>>` (redirección añadiendo).
    RedirectAppend,
    /// Operador `|` (tubería).
    Pipe,
}

/// Estado del tokenizador respecto a comillas.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Quote {
    No,
    Single,
    Double,
}

/// Tokeniza una línea en [`Token`]s, respetando comillas y escapes.
///
/// Reglas:
/// - Espacios/tabuladores separan palabras (fuera de comillas).
/// - `'…'`: todo literal hasta la siguiente comilla simple (ni escapes).
/// - `"…"`: literal, pero `\"` y `\\` se interpretan como `"` y `\`.
/// - `\c` fuera de comillas: inserta `c` literalmente (permite espacios en
///   argumentos, p. ej. `a\ b` es la palabra `a b`).
/// - `>` / `>>` y `|` son operadores incluso pegados a una palabra
///   (`echo hi>out` ⇒ `echo`, `hi`, `>`, `out`).
///
/// Devuelve `Err(KError::InvalidArgument)` si queda una comilla sin cerrar.
pub fn tokenize(line: &str) -> KResult<Vec<Token>> {
    let mut tokens: Vec<Token> = Vec::new();
    let mut current = String::new();
    // `has_word` distingue "no hay palabra en curso" de "palabra vacía en
    // curso" (necesario para que `""` produzca una palabra vacía).
    let mut has_word = false;
    let mut quote = Quote::No;

    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match quote {
            Quote::Single => {
                if c == '\'' {
                    quote = Quote::No;
                } else {
                    current.push(c);
                }
                has_word = true;
            }
            Quote::Double => {
                if c == '"' {
                    quote = Quote::No;
                } else if c == '\\' {
                    // En comillas dobles sólo `\"` y `\\` son escapes; el resto
                    // de barras se conservan literales.
                    match chars.peek() {
                        Some(&n) if n == '"' || n == '\\' => {
                            current.push(n);
                            chars.next();
                        }
                        _ => current.push('\\'),
                    }
                } else {
                    current.push(c);
                }
                has_word = true;
            }
            Quote::No => match c {
                ' ' | '\t' | '\r' | '\n' => {
                    if has_word {
                        tokens.push(Token::Word(core::mem::take(&mut current)));
                        has_word = false;
                    }
                }
                '\'' => {
                    quote = Quote::Single;
                    has_word = true;
                }
                '"' => {
                    quote = Quote::Double;
                    has_word = true;
                }
                '\\' => {
                    // Escapa el siguiente carácter literalmente.
                    match chars.next() {
                        Some(n) => current.push(n),
                        None => current.push('\\'),
                    }
                    has_word = true;
                }
                '|' => {
                    if has_word {
                        tokens.push(Token::Word(core::mem::take(&mut current)));
                        has_word = false;
                    }
                    tokens.push(Token::Pipe);
                }
                '>' => {
                    if has_word {
                        tokens.push(Token::Word(core::mem::take(&mut current)));
                        has_word = false;
                    }
                    if let Some(&'>') = chars.peek() {
                        chars.next();
                        tokens.push(Token::RedirectAppend);
                    } else {
                        tokens.push(Token::RedirectOut);
                    }
                }
                _ => {
                    current.push(c);
                    has_word = true;
                }
            },
        }
    }

    if quote != Quote::No {
        // Comilla sin cerrar: entrada malformada.
        return Err(KError::InvalidArgument);
    }
    if has_word {
        tokens.push(Token::Word(current));
    }
    Ok(tokens)
}

/// Construye un [`Command`] a partir de las palabras y la redirección
/// acumuladas de una etapa. Consume (vacía) ambos acumuladores.
///
/// Devuelve `Err(KError::InvalidArgument)` si no hay ninguna palabra (etapa
/// vacía, p. ej. tubería sin comando: `| ls` o `ls |`).
fn build_command(words: &mut Vec<String>, redirect: &mut Redirect) -> KResult<Command> {
    if words.is_empty() {
        return Err(KError::InvalidArgument);
    }
    // `remove(0)` es seguro: acabamos de comprobar que no está vacío.
    let name = words.remove(0);
    let args = core::mem::take(words);
    let redir = core::mem::replace(redirect, Redirect::None);
    Ok(Command {
        name,
        args,
        redirect: redir,
    })
}

/// Parsea la línea como una tubería completa: una etapa [`Command`] por cada
/// sección separada por `|`.
///
/// - Línea vacía o sólo espacios ⇒ `Ok(vec![])`.
/// - Comilla sin cerrar, redirección sin archivo, o etapa vacía ⇒
///   `Err(KError::InvalidArgument)`.
pub fn parse_pipeline(line: &str) -> KResult<Vec<Command>> {
    let tokens = tokenize(line)?;
    if tokens.is_empty() {
        return Ok(Vec::new());
    }

    let mut commands: Vec<Command> = Vec::new();
    let mut words: Vec<String> = Vec::new();
    let mut redirect = Redirect::None;
    // Operador de redirección a la espera de su nombre de archivo:
    //   None            => ninguno pendiente
    //   Some(false)     => `>`  (truncar)
    //   Some(true)      => `>>` (añadir)
    let mut pending: Option<bool> = None;

    for tok in tokens {
        // Si esperamos el archivo de una redirección, el siguiente token debe
        // ser una palabra.
        if let Some(append) = pending {
            match tok {
                Token::Word(w) => {
                    redirect = if append {
                        Redirect::Append(w)
                    } else {
                        Redirect::Truncate(w)
                    };
                    pending = None;
                    continue;
                }
                _ => return Err(KError::InvalidArgument),
            }
        }

        match tok {
            Token::Word(w) => words.push(w),
            Token::RedirectOut => pending = Some(false),
            Token::RedirectAppend => pending = Some(true),
            Token::Pipe => {
                let cmd = build_command(&mut words, &mut redirect)?;
                commands.push(cmd);
            }
        }
    }

    if pending.is_some() {
        // Redirección al final de la línea sin nombre de archivo.
        return Err(KError::InvalidArgument);
    }

    let last = build_command(&mut words, &mut redirect)?;
    commands.push(last);
    Ok(commands)
}

/// Parsea una línea. `Ok(None)` = línea vacía / solo espacios. [CANÓNICO]
///
/// Respeta comillas simples/dobles y detecta `>`, `>>` y `|`. Devuelve la
/// PRIMERA etapa de la tubería (normalmente la única). Para operar con todas
/// las etapas, la shell usa [`parse_pipeline`].
pub fn parse(line: &str) -> KResult<Option<Command>> {
    let mut pipeline = parse_pipeline(line)?;
    if pipeline.is_empty() {
        Ok(None)
    } else {
        // `remove(0)` seguro: la tubería no está vacía.
        Ok(Some(pipeline.remove(0)))
    }
}

#[cfg(test)]
mod tests {
    //! Tests unitarios del tokenizador y el parser.
    //!
    //! Son lógica pura sobre `&str`; se ejecutan en el host (`cargo test`).
    //! No forman parte de la compilación del firmware (`cfg(test)` off).
    use super::{parse, parse_pipeline, tokenize, Command, Redirect, Token};
    use crate::prelude::*;
    use alloc::vec;

    fn w(s: &str) -> Token {
        Token::Word(String::from(s))
    }

    #[test]
    fn tokeniza_palabras_simples() {
        assert_eq!(
            tokenize("echo hola mundo"),
            Ok(vec![w("echo"), w("hola"), w("mundo")])
        );
    }

    #[test]
    fn colapsa_espacios_repetidos() {
        assert_eq!(
            tokenize("  ls    -la  "),
            Ok(vec![w("ls"), w("-la")])
        );
    }

    #[test]
    fn respeta_comillas_dobles() {
        assert_eq!(
            tokenize("echo \"hola   mundo\""),
            Ok(vec![w("echo"), w("hola   mundo")])
        );
    }

    #[test]
    fn respeta_comillas_simples() {
        assert_eq!(
            tokenize("echo 'a | b > c'"),
            Ok(vec![w("echo"), w("a | b > c")])
        );
    }

    #[test]
    fn comillas_vacias_producen_palabra_vacia() {
        assert_eq!(tokenize("echo \"\""), Ok(vec![w("echo"), w("")]));
    }

    #[test]
    fn concatena_partes_entrecomilladas() {
        assert_eq!(tokenize("a\"b c\"d"), Ok(vec![w("ab cd")]));
    }

    #[test]
    fn escape_fuera_de_comillas() {
        assert_eq!(tokenize("a\\ b"), Ok(vec![w("a b")]));
    }

    #[test]
    fn escape_en_comillas_dobles() {
        assert_eq!(tokenize("\"a\\\"b\""), Ok(vec![w("a\"b")]));
    }

    #[test]
    fn detecta_operadores_pegados() {
        assert_eq!(
            tokenize("echo hi>out"),
            Ok(vec![w("echo"), w("hi"), Token::RedirectOut, w("out")])
        );
    }

    #[test]
    fn distingue_append_de_truncate() {
        assert_eq!(
            tokenize("cat a >> b"),
            Ok(vec![w("cat"), w("a"), Token::RedirectAppend, w("b")])
        );
    }

    #[test]
    fn detecta_tuberia() {
        assert_eq!(
            tokenize("ls | cat"),
            Ok(vec![w("ls"), Token::Pipe, w("cat")])
        );
    }

    #[test]
    fn comilla_sin_cerrar_es_error() {
        assert_eq!(tokenize("echo \"abc"), Err(KError::InvalidArgument));
    }

    #[test]
    fn parse_comando_con_redireccion() {
        let cmd = parse("echo hola > /tmp/a.txt").unwrap().unwrap();
        assert_eq!(
            cmd,
            Command {
                name: String::from("echo"),
                args: vec![String::from("hola")],
                redirect: Redirect::Truncate(String::from("/tmp/a.txt")),
            }
        );
    }

    #[test]
    fn parse_append() {
        let cmd = parse("cat log >> /tmp/all").unwrap().unwrap();
        assert_eq!(cmd.redirect, Redirect::Append(String::from("/tmp/all")));
    }

    #[test]
    fn parse_linea_vacia_es_none() {
        assert_eq!(parse("   \t "), Ok(None));
    }

    #[test]
    fn pipeline_dos_etapas() {
        let stages = parse_pipeline("ls -l | cat").unwrap();
        assert_eq!(stages.len(), 2);
        assert_eq!(stages[0].name, "ls");
        assert_eq!(stages[0].args, vec![String::from("-l")]);
        assert_eq!(stages[1].name, "cat");
    }

    #[test]
    fn redireccion_sin_archivo_es_error() {
        assert_eq!(parse_pipeline("echo hi >"), Err(KError::InvalidArgument));
    }

    #[test]
    fn tuberia_con_etapa_vacia_es_error() {
        assert_eq!(parse_pipeline("| ls"), Err(KError::InvalidArgument));
        assert_eq!(parse_pipeline("ls |"), Err(KError::InvalidArgument));
    }
}
