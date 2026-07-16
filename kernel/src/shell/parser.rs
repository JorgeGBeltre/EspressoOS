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

    /// `;` -- the boundary between two pipelines that run one after the other.
    ///
    /// A token and not a `line.split(';')` before tokenizing, because splitting the
    /// raw string would cut `echo "a;b"` in half. Protecting a separator inside quotes
    /// is the entire reason this tokenizer exists; a second separator that ignored it
    /// would be a hole in the thing it is for.
    Semi,
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
                ';' => {
                    if has_word {
                        tokens.push(Token::Word(core::mem::take(&mut current)));
                        has_word = false;
                    }
                    tokens.push(Token::Semi);
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

/// One pipeline's worth of tokens -> its stages. `Semi` never reaches here; parse_line
/// is what splits on it.
fn pipeline_from_tokens(tokens: Vec<Token>) -> KResult<Vec<Command>> {
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
            // parse_line strips every Semi before calling this, so reaching one means
            // the two disagree. Report it rather than assume; the arm has to exist
            // either way.
            Token::Semi => return Err(KError::InvalidArgument),
        }
    }

    if pending.is_some() {
        return Err(KError::InvalidArgument);
    }

    let last = build_command(&mut words, &mut redirect)?;
    commands.push(last);
    Ok(commands)
}

/// A whole line -> the pipelines it asks for, in order.
///
/// This is the entry point the shell uses. `;` separates pipelines that run one after
/// the other; `|` separates the stages within one. Both are recognized by the
/// tokenizer, so quoting protects both: `echo "a;b"` is one word and `echo 'a|b'` is
/// too.
///
/// Before this existed, `;` was not a token at all and fell through to the word arm --
/// so `mkdir a ; cd a` asked mkdir to create three directories, one of them named ";",
/// and reported nothing. A shell that turns a standard separator into data silently is
/// worse than one that has never heard of it.
pub fn parse_line(line: &str) -> KResult<Vec<Vec<Command>>> {
    let mut out: Vec<Vec<Command>> = Vec::new();
    let mut segment: Vec<Token> = Vec::new();

    for tok in tokenize(line)? {
        if tok == Token::Semi {
            // An empty segment separates nothing: `ls ;` is one command, and `ls ;; cat`
            // is two. Skipped rather than treated as a syntax error, which is what every
            // shell does and what makes a trailing `;` harmless.
            if !segment.is_empty() {
                out.push(pipeline_from_tokens(core::mem::take(&mut segment))?);
            }
        } else {
            segment.push(tok);
        }
    }
    if !segment.is_empty() {
        out.push(pipeline_from_tokens(segment)?);
    }
    Ok(out)
}

/// One pipeline, rejecting `;`. Kept for the callers that want exactly one.
pub fn parse_pipeline(line: &str) -> KResult<Vec<Command>> {
    pipeline_from_tokens(tokenize(line)?)
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

    // These, like every test in this file, never run: the kernel is a no_std binary
    // with no lib target. tools/tests/logic_tests.py mirrors them and is the one that
    // actually executes. Keep the two in step.

    #[test]
    fn semi_separa_ordenes() {
        let pipes = parse_line("mkdir a ; cd a").unwrap();
        assert_eq!(pipes.len(), 2);
        assert_eq!(pipes[0][0].name, "mkdir");
        assert_eq!(pipes[0][0].args, vec![String::from("a")]);
        assert_eq!(pipes[1][0].name, "cd");
    }

    #[test]
    fn semi_sin_espacios() {
        let pipes = parse_line("pwd;ls").unwrap();
        assert_eq!(pipes.len(), 2);
    }

    /// Why Semi is a token and not a `line.split(';')` before tokenizing. With a split,
    /// this would become two commands and echo would be handed a stray quote.
    #[test]
    fn las_comillas_protegen_el_semi() {
        let pipes = parse_line("echo \"a;b\"").unwrap();
        assert_eq!(pipes.len(), 1);
        assert_eq!(pipes[0][0].args, vec![String::from("a;b")]);
        let pipes = parse_line("echo 'x ; y'").unwrap();
        assert_eq!(pipes.len(), 1);
        assert_eq!(pipes[0][0].args, vec![String::from("x ; y")]);
    }

    #[test]
    fn semi_final_o_repetido_es_inofensivo() {
        assert_eq!(parse_line("ls ;").unwrap().len(), 1);
        assert_eq!(parse_line("ls ;; pwd").unwrap().len(), 2);
        assert_eq!(parse_line(" ; ls").unwrap().len(), 1);
        assert!(parse_line(";;;").unwrap().is_empty());
    }

    #[test]
    fn semi_convive_con_tuberias_y_redirecciones() {
        let pipes = parse_line("ls | cat ; echo hi > f").unwrap();
        assert_eq!(pipes.len(), 2);
        assert_eq!(pipes[0].len(), 2);
        assert_eq!(pipes[1][0].redirect, Redirect::Truncate(String::from("f")));
    }

    /// The whole line is parsed before any of it runs, so `rm x ; echo >` must not
    /// delete x and then complain.
    #[test]
    fn error_en_una_orden_invalida_toda_la_linea() {
        assert_eq!(parse_line("rm x ; echo >"), Err(KError::InvalidArgument));
    }

    #[test]
    fn parse_pipeline_rechaza_el_semi() {
        assert_eq!(parse_pipeline("ls ; cat"), Err(KError::InvalidArgument));
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
