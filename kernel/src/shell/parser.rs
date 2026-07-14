#![allow(dead_code)]

use crate::prelude::*;

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Redirect {
    None,

    Truncate(String),

    Append(String),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Command {
    pub name: String,
    pub args: Vec<String>,
    pub redirect: Redirect,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Token {
    Word(String),

    RedirectOut,

    RedirectAppend,

    Pipe,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Quote {
    No,
    Single,
    Double,
}

pub fn tokenize(line: &str) -> KResult<Vec<Token>> {
    let mut tokens: Vec<Token> = Vec::new();
    let mut current = String::new();

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
        return Err(KError::InvalidArgument);
    }
    if has_word {
        tokens.push(Token::Word(current));
    }
    Ok(tokens)
}

fn build_command(words: &mut Vec<String>, redirect: &mut Redirect) -> KResult<Command> {
    if words.is_empty() {
        return Err(KError::InvalidArgument);
    }

    let name = words.remove(0);
    let args = core::mem::take(words);
    let redir = core::mem::replace(redirect, Redirect::None);
    Ok(Command {
        name,
        args,
        redirect: redir,
    })
}

pub fn parse_pipeline(line: &str) -> KResult<Vec<Command>> {
    let tokens = tokenize(line)?;
    if tokens.is_empty() {
        return Ok(Vec::new());
    }

    let mut commands: Vec<Command> = Vec::new();
    let mut words: Vec<String> = Vec::new();
    let mut redirect = Redirect::None;

    let mut pending: Option<bool> = None;

    for tok in tokens {
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
        return Err(KError::InvalidArgument);
    }

    let last = build_command(&mut words, &mut redirect)?;
    commands.push(last);
    Ok(commands)
}

pub fn parse(line: &str) -> KResult<Option<Command>> {
    let mut pipeline = parse_pipeline(line)?;
    if pipeline.is_empty() {
        Ok(None)
    } else {
        Ok(Some(pipeline.remove(0)))
    }
}

#[cfg(test)]
mod tests {

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
        assert_eq!(tokenize("  ls    -la  "), Ok(vec![w("ls"), w("-la")]));
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
