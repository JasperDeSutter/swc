use super::{
    state::{lex, lex_module, lex_tokens, with_lexer},
    *,
};
use error::{Error, SyntaxError};
use std::{ops::Range, str};
use test::Bencher;

fn sp(r: Range<usize>) -> Span {
    Span::new(
        BytePos(r.start as u32),
        BytePos(r.end as u32),
        Default::default(),
    )
}

trait LineBreak: Into<TokenAndSpan> {
    fn lb(self) -> TokenAndSpan {
        TokenAndSpan {
            had_line_break: true,
            ..self.into()
        }
    }
}
impl LineBreak for TokenAndSpan {}

trait SpanRange: Sized {
    fn into_span(self) -> Span;
}
impl SpanRange for usize {
    fn into_span(self) -> Span {
        Span::new(
            BytePos(self as _),
            BytePos((self + 1usize) as _),
            Default::default(),
        )
    }
}
impl SpanRange for Range<usize> {
    fn into_span(self) -> Span {
        Span::new(
            BytePos(self.start as _),
            BytePos(self.end as _),
            Default::default(),
        )
    }
}

trait WithSpan: Sized {
    fn span<R>(self, span: R) -> TokenAndSpan
    where
        R: SpanRange,
    {
        TokenAndSpan {
            token: self.into_token(),
            had_line_break: false,
            span: span.into_span(),
        }
    }
    fn into_token(self) -> Token;
}
impl WithSpan for Token {
    fn into_token(self) -> Token {
        self
    }
}
impl WithSpan for usize {
    fn into_token(self) -> Token {
        Num(self as f64)
    }
}
impl WithSpan for f64 {
    fn into_token(self) -> Token {
        Num(self)
    }
}
impl<'a> WithSpan for &'a str {
    fn into_token(self) -> Token {
        Word(Word::Ident(self.into()))
    }
}
impl WithSpan for Keyword {
    fn into_token(self) -> Token {
        Word(Word::Keyword(self))
    }
}
impl WithSpan for Word {
    fn into_token(self) -> Token {
        Word(self)
    }
}
impl WithSpan for BinOpToken {
    fn into_token(self) -> Token {
        BinOp(self)
    }
}
impl WithSpan for AssignOpToken {
    fn into_token(self) -> Token {
        AssignOp(self)
    }
}

#[test]
fn module_legacy_octal() {
    assert_eq!(
        lex_module(Syntax::default(), "01"),
        vec![Token::Error(Error {
            span: sp(0..2),
            error: SyntaxError::LegacyOctal,
        })
        .span(0..2)
        .lb(),]
    );
}
#[test]
fn module_legacy_decimal() {
    assert_eq!(
        lex_module(Syntax::default(), "08"),
        vec![Token::Error(Error {
            span: sp(0..2),
            error: SyntaxError::LegacyDecimal,
        })
        .span(0..2)
        .lb(),]
    );
}

#[test]
fn module_legacy_comment_1() {
    assert_eq!(
        lex_module(Syntax::default(), "<!-- foo oo"),
        vec![Token::Error(Error {
            span: sp(0..11),
            error: SyntaxError::LegacyCommentInModule,
        })
        .span(0..11)
        .lb(),]
    )
}

#[test]
fn module_legacy_comment_2() {
    assert_eq!(
        lex_module(Syntax::default(), "-->"),
        vec![Token::Error(Error {
            span: sp(0..3),
            error: SyntaxError::LegacyCommentInModule,
        })
        .span(0..3)
        .lb(),]
    )
}

#[test]
fn test262_lexer_error_0001() {
    assert_eq!(
        lex(Syntax::default(), "123..a(1)"),
        vec![
            123f64.span(0..4).lb(),
            Dot.span(4..5),
            "a".span(5..6),
            LParen.span(6..7),
            1.span(7..8),
            RParen.span(8..9),
        ],
    )
}

#[test]
fn test262_lexer_error_0002() {
    assert_eq!(
        lex(Syntax::default(), r#"'use\x20strict';"#),
        vec![
            Token::Str {
                value: "use strict".into(),
                has_escape: true,
            }
            .span(0..15)
            .lb(),
            Semi.span(15..16),
        ]
    );
}

#[test]
fn test262_lexer_error_0003() {
    assert_eq!(
        lex(Syntax::default(), r#"\u0061"#),
        vec!["a".span(0..6).lb()]
    );
}

#[test]
fn test262_lexer_error_0004() {
    assert_eq!(
        lex_tokens(Syntax::default(), "+{} / 1"),
        vec![tok!('+'), tok!('{'), tok!('}'), tok!('/'), 1.into_token()]
    );
}

#[test]
fn ident_escape_unicode() {
    assert_eq!(
        lex(Syntax::default(), r#"a\u0061"#),
        vec!["aa".span(0..7).lb()]
    );
}

#[test]
fn ident_escape_unicode_2() {
    assert_eq!(lex(Syntax::default(), "℘℘"), vec!["℘℘".span(0..6).lb()]);

    assert_eq!(
        lex(Syntax::default(), r#"℘\u2118"#),
        vec!["℘℘".span(0..9).lb()]
    );
}

#[test]
fn tpl_multiline() {
    assert_eq!(
        lex_tokens(
            Syntax::default(),
            "`this
is
multiline`"
        ),
        vec![
            tok!('`'),
            Token::Template {
                cooked: "this\nis\nmultiline".into(),
                raw: "this\\nis\\nmultiline".into(),
                has_escape: false
            },
            tok!('`'),
        ]
    );
}

#[test]
fn tpl_raw_unicode_escape() {
    assert_eq!(
        lex_tokens(Syntax::default(), r"`\u{0010}`"),
        vec![
            tok!('`'),
            Token::Template {
                cooked: format!("{}", '\u{0010}').into(),
                raw: "\\u{0010}".into(),
                has_escape: true
            },
            tok!('`'),
        ]
    );
}

#[test]
fn str_escape() {
    assert_eq!(
        lex_tokens(Syntax::default(), r#"'\n'"#),
        vec![Token::Str {
            value: "\n".into(),
            has_escape: true
        }]
    );
}

#[test]
fn str_escape_2() {
    assert_eq!(
        lex_tokens(Syntax::default(), r#"'\\n'"#),
        vec![Token::Str {
            value: "\\n".into(),
            has_escape: true
        }]
    );
}

#[test]
fn str_escape_hex() {
    assert_eq!(
        lex(Syntax::default(), r#"'\x61'"#),
        vec![Token::Str {
            value: "a".into(),
            has_escape: true,
        }
        .span(0..6)
        .lb(),]
    );
}

#[test]
fn str_escape_octal() {
    assert_eq!(
        lex(Syntax::default(), r#"'Hello\012World'"#),
        vec![Token::Str {
            value: "Hello\nWorld".into(),
            has_escape: true,
        }
        .span(0..16)
        .lb(),]
    )
}

#[test]
fn str_escape_unicode_long() {
    assert_eq!(
        lex(Syntax::default(), r#"'\u{00000000034}'"#),
        vec![Token::Str {
            value: "4".into(),
            has_escape: true,
        }
        .span(0..17)
        .lb(),]
    );
}

#[test]
fn regexp_unary_void() {
    assert_eq!(
        lex(Syntax::default(), "void /test/"),
        vec![
            Void.span(0..4).lb(),
            Regex(
                Str {
                    value: "test".into(),
                    span: sp(6..10),
                    has_escape: false,
                },
                None,
            )
            .span(5..11),
        ]
    );
    assert_eq!(
        lex(Syntax::default(), "void (/test/)"),
        vec![
            Void.span(0..4).lb(),
            LParen.span(5..6),
            Regex(
                Str {
                    span: sp(7..11),
                    value: "test".into(),
                    has_escape: false,
                },
                None,
            )
            .span(6..12),
            RParen.span(12..13),
        ]
    );
}

#[test]
fn non_regexp_unary_plus() {
    assert_eq!(
        lex(Syntax::default(), "+{} / 1"),
        vec![
            tok!('+').span(0..1).lb(),
            tok!('{').span(1..2),
            tok!('}').span(2..3),
            tok!('/').span(4..5),
            1.span(6..7),
        ]
    );
}

// ----------

#[test]
fn paren_semi() {
    assert_eq!(
        lex(Syntax::default(), "();"),
        vec![LParen.span(0).lb(), RParen.span(1), Semi.span(2)]
    );
}

#[test]
fn ident_paren() {
    assert_eq!(
        lex(Syntax::default(), "a(bc);"),
        vec![
            "a".span(0).lb(),
            LParen.span(1),
            "bc".span(2..4),
            RParen.span(4),
            Semi.span(5),
        ]
    );
}

#[test]
fn read_word() {
    assert_eq!(
        lex(Syntax::default(), "a b c"),
        vec!["a".span(0).lb(), "b".span(2), "c".span(4)]
    )
}

#[test]
fn simple_regex() {
    assert_eq!(
        lex(Syntax::default(), "x = /42/i"),
        vec![
            "x".span(0).lb(),
            Assign.span(2),
            Regex(
                Str {
                    span: sp(5..7),
                    value: "42".into(),
                    has_escape: false,
                },
                Some(Str {
                    span: sp(8..9),
                    value: "i".into(),
                    has_escape: false,
                }),
            )
            .span(4..9),
        ],
    );

    assert_eq!(
        lex(Syntax::default(), "/42/"),
        vec![Regex(
            Str {
                span: sp(1..3),
                value: "42".into(),
                has_escape: false,
            },
            None,
        )
        .span(0..4)
        .lb(),]
    );
}

#[test]
fn complex_regex() {
    assert_eq_ignore_span!(
        lex_tokens(Syntax::default(), "f(); function foo() {} /42/i"),
        vec![
            Word(Word::Ident("f".into())),
            LParen,
            RParen,
            Semi,
            Word(Word::Keyword(Function)),
            Word(Word::Ident("foo".into())),
            LParen,
            RParen,
            LBrace,
            RBrace,
            Regex(
                Str {
                    span: Default::default(),
                    value: "42".into(),
                    has_escape: false,
                },
                Some(Str {
                    span: Default::default(),
                    value: "i".into(),
                    has_escape: false,
                }),
            ),
        ]
    )
}

#[test]
fn simple_div() {
    assert_eq!(
        lex(Syntax::default(), "a / b"),
        vec!["a".span(0).lb(), Div.span(2), "b".span(4)],
    );
}

#[test]
fn complex_divide() {
    assert_eq!(
        lex_tokens(Syntax::default(), "x = function foo() {} /a/i"),
        vec![
            Word(Word::Ident("x".into())),
            AssignOp(Assign),
            Word(Word::Keyword(Function)),
            Word(Word::Ident("foo".into())),
            LParen,
            RParen,
            LBrace,
            RBrace,
            BinOp(Div),
            Word(Word::Ident("a".into())),
            BinOp(Div),
            Word(Word::Ident("i".into())),
        ],
        "/ should be parsed as div operator"
    )
}

// ---------- Tests from tc39 spec

#[test]
fn spec_001() {
    let expected = vec![
        Word(Word::Ident("a".into())),
        AssignOp(Assign),
        Word(Word::Ident("b".into())),
        BinOp(Div),
        Word(Word::Ident("hi".into())),
        BinOp(Div),
        Word(Word::Ident("g".into())),
        Dot,
        Word(Word::Ident("exec".into())),
        LParen,
        Word(Word::Ident("c".into())),
        RParen,
        Dot,
        Word(Word::Ident("map".into())),
        LParen,
        Word(Word::Ident("d".into())),
        RParen,
        Semi,
    ];

    assert_eq!(
        lex_tokens(
            Syntax::default(),
            "a = b
/hi/g.exec(c).map(d);"
        ),
        expected
    );
    assert_eq!(
        lex_tokens(Syntax::default(), "a = b / hi / g.exec(c).map(d);"),
        expected
    );
}

// ---------- Tests ported from esprima

#[test]
fn after_if() {
    assert_eq!(
        lex(Syntax::default(), "if(x){} /y/.test(z)"),
        vec![
            Keyword::If.span(0..2).lb(),
            LParen.span(2),
            "x".span(3),
            RParen.span(4),
            LBrace.span(5),
            RBrace.span(6),
            Regex(
                Str {
                    span: sp(9..10),
                    value: "y".into(),
                    has_escape: false,
                },
                None,
            )
            .span(8..11),
            Dot.span(11),
            "test".span(12..16),
            LParen.span(16),
            "z".span(17),
            RParen.span(18),
        ],
    )
}

// #[test]
// #[ignore]
// fn leading_comment() {
//     assert_eq!(
//         vec![
//             BlockComment(" hello world ".into()).span(0..17),
//             Regex("42".into(), "".into()).span(17..21),
//         ],
//         lex(Syntax::default(), "/* hello world */  /42/")
//     )
// }

// #[test]
// #[ignore]
// fn line_comment() {
//     assert_eq!(
//         vec![
//             Keyword::Var.span(0..3),
//             "answer".span(4..10),
//             Assign.span(11),
//             42.span(13..15),
//             LineComment(" the Ultimate".into()).span(17..32),
//         ],
//         lex(Syntax::default(), "var answer = 42  // the Ultimate"),
//     )
// }

#[test]
fn migrated_0002() {
    assert_eq!(
        lex(Syntax::default(), "tokenize(/42/)"),
        vec![
            "tokenize".span(0..8).lb(),
            LParen.span(8),
            Regex(
                Str {
                    span: sp(10..12),
                    value: "42".into(),
                    has_escape: false,
                },
                None,
            )
            .span(9..13),
            RParen.span(13),
        ],
    )
}

#[test]
fn migrated_0003() {
    assert_eq!(
        lex(Syntax::default(), "(false) /42/"),
        vec![
            LParen.span(0).lb(),
            Word::False.span(1..6),
            RParen.span(6),
            Div.span(8),
            42.span(9..11),
            Div.span(11),
        ],
    )
}

#[test]
fn migrated_0004() {
    assert_eq!(
        lex(Syntax::default(), "function f(){} /42/"),
        vec![
            Function.span(0..8).lb(),
            "f".span(9),
            LParen.span(10),
            RParen.span(11),
            LBrace.span(12),
            RBrace.span(13),
            Regex(
                Str {
                    span: sp(16..18),
                    value: "42".into(),
                    has_escape: false,
                },
                None,
            )
            .span(15..19),
        ]
    );
}

// This test seems wrong.
//
// #[test]
// fn migrated_0005() {
//     assert_eq!(
//         vec![
//             Function.span(0..8),
//             LParen.span(9),
//             RParen.span(10),
//             LBrace.span(11),
//             RBrace.span(12),
//             Div.span(13),
//             42.span(14..16),
//         ],
//         lex(Syntax::default(), "function (){} /42")
//     );
// }

#[test]
fn migrated_0006() {
    // This test seems wrong.
    // assert_eq!(
    //     vec![LBrace.span(0).lb(), RBrace.span(1), Div.span(3), 42.span(4..6)],
    //     lex(Syntax::default(), "{} /42")
    // )

    assert_eq!(
        lex(Syntax::default(), "{} /42/"),
        vec![
            LBrace.span(0).lb(),
            RBrace.span(1),
            Regex(
                Str {
                    span: sp(4..6),
                    value: "42".into(),
                    has_escape: false,
                },
                None,
            )
            .span(3..7),
        ],
    )
}

#[test]
fn str_lit() {
    assert_eq!(
        lex_tokens(Syntax::default(), "'abcde'"),
        vec![Token::Str {
            value: "abcde".into(),
            has_escape: false,
        }],
    );
    assert_eq_ignore_span!(
        lex_tokens(Syntax::default(), "'\\\nabc'"),
        vec![Token::Str {
            value: "abc".into(),
            has_escape: true,
        }]
    );
}

#[test]
fn tpl_empty() {
    assert_eq!(
        lex_tokens(Syntax::default(), r#"``"#),
        vec![
            tok!('`'),
            Template {
                raw: "".into(),
                cooked: "".into(),
                has_escape: false
            },
            tok!('`')
        ]
    )
}

#[test]
fn tpl() {
    assert_eq!(
        lex_tokens(Syntax::default(), r#"`${a}`"#),
        vec![
            tok!('`'),
            Template {
                raw: "".into(),
                cooked: "".into(),
                has_escape: false
            },
            tok!("${"),
            Word(Word::Ident("a".into())),
            tok!('}'),
            Template {
                raw: "".into(),
                cooked: "".into(),
                has_escape: false
            },
            tok!('`'),
        ]
    )
}

#[test]
fn comment() {
    assert_eq!(
        lex(
            Syntax::default(),
            "// foo
a"
        ),
        vec![TokenAndSpan {
            token: Word(Word::Ident("a".into())),
            span: sp(7..8),
            // first line
            had_line_break: true
        }]
    );
}

#[test]
fn comment_2() {
    assert_eq!(
        lex(
            Syntax::default(),
            "// foo
// bar
a"
        ),
        vec![TokenAndSpan {
            token: Word(Word::Ident("a".into())),
            span: sp(14..15),
            // first line
            had_line_break: true
        }]
    );
}

#[test]
fn jsx_01() {
    assert_eq!(
        lex_tokens(
            ::Syntax::Es(::EsConfig {
                jsx: true,
                ..Default::default()
            }),
            "<a />"
        ),
        vec![
            Token::JSXTagStart,
            Token::JSXName { name: "a".into() },
            tok!('/'),
            Token::JSXTagEnd,
        ]
    );
}

#[test]
fn jsx_02() {
    assert_eq!(
        lex_tokens(
            ::Syntax::Es(::EsConfig {
                jsx: true,
                ..Default::default()
            }),
            "<a>foo</a>"
        ),
        vec![
            Token::JSXTagStart,
            Token::JSXName { name: "a".into() },
            Token::JSXTagEnd,
            Token::JSXText { raw: "foo".into() },
            Token::JSXTagStart,
            tok!('/'),
            Token::JSXName { name: "a".into() },
            Token::JSXTagEnd,
        ]
    );
}

#[test]
fn jsx_03() {
    assert_eq!(
        lex_tokens(
            ::Syntax::Es(::EsConfig {
                jsx: true,
                ..Default::default()
            }),
            "<a><br /></a>"
        ),
        vec![
            Token::JSXTagStart,
            Token::JSXName { name: "a".into() },
            Token::JSXTagEnd,
            //
            Token::JSXTagStart,
            Token::JSXName { name: "br".into() },
            tok!('/'),
            Token::JSXTagEnd,
            //
            Token::JSXTagStart,
            tok!('/'),
            Token::JSXName { name: "a".into() },
            Token::JSXTagEnd,
        ]
    );
}

#[test]
fn jsx_04() {
    assert_eq!(
        lex_tokens(
            ::Syntax::Es(::EsConfig {
                jsx: true,
                ..Default::default()
            }),
            "yield <a></a>"
        ),
        vec![
            Token::Word(Word::Keyword(Yield)),
            Token::JSXTagStart,
            Token::JSXName { name: "a".into() },
            Token::JSXTagEnd,
            //
            Token::JSXTagStart,
            tok!('/'),
            Token::JSXName { name: "a".into() },
            Token::JSXTagEnd,
        ]
    );
}

#[test]
fn max_integer() {
    lex_tokens(::Syntax::default(), "1.7976931348623157e+308");
}

#[test]
fn shebang() {
    assert_eq!(
        lex_tokens(::Syntax::default(), "#!/usr/bin/env node",),
        vec![Token::Shebang("/usr/bin/env node".into())]
    );
}

#[test]
fn empty() {
    assert_eq!(lex_tokens(::Syntax::default(), "",), vec![]);
}

#[test]
fn issue_191() {
    assert_eq!(
        lex_tokens(
            ::Syntax::Es(::EsConfig {
                jsx: true,
                ..Default::default()
            }),
            "`${foo}<bar>`"
        ),
        vec![
            tok!('`'),
            Token::Template {
                raw: "".into(),
                cooked: "".into(),
                has_escape: false,
            },
            tok!("${"),
            Token::Word(Word::Ident("foo".into())),
            tok!('}'),
            Token::Template {
                raw: "<bar>".into(),
                cooked: "<bar>".into(),
                has_escape: false,
            },
            tok!('`')
        ]
    );
}

#[test]
fn jsx_05() {
    assert_eq!(
        lex_tokens(
            ::Syntax::Es(::EsConfig {
                jsx: true,
                ..Default::default()
            }),
            "<a>{}</a>"
        ),
        vec![
            Token::JSXTagStart,
            Token::JSXName { name: "a".into() },
            Token::JSXTagEnd,
            //
            tok!('{'),
            tok!('}'),
            //
            Token::JSXTagStart,
            tok!('/'),
            Token::JSXName { name: "a".into() },
            Token::JSXTagEnd,
        ]
    );
}

#[test]
fn issue_299_01() {
    assert_eq!(
        lex_tokens(
            ::Syntax::Es(::EsConfig {
                jsx: true,
                ..Default::default()
            }),
            "<Page num='\\ '>ABC</Page>;"
        ),
        vec![
            Token::JSXTagStart,
            Token::JSXName {
                name: "Page".into()
            },
            Token::JSXName { name: "num".into() },
            tok!('='),
            Token::Str {
                value: " ".into(),
                has_escape: true
            },
            Token::JSXTagEnd,
            JSXText { raw: "ABC".into() },
            JSXTagStart,
            tok!('/'),
            JSXName {
                name: "Page".into()
            },
            JSXTagEnd,
            Semi,
        ]
    );
}

#[test]
fn issue_299_02() {
    assert_eq!(
        lex_tokens(
            ::Syntax::Es(::EsConfig {
                jsx: true,
                ..Default::default()
            }),
            "<Page num='\\''>ABC</Page>;"
        ),
        vec![
            Token::JSXTagStart,
            Token::JSXName {
                name: "Page".into()
            },
            Token::JSXName { name: "num".into() },
            tok!('='),
            Token::Str {
                value: "'".into(),
                has_escape: true
            },
            Token::JSXTagEnd,
            JSXText { raw: "ABC".into() },
            JSXTagStart,
            tok!('/'),
            JSXName {
                name: "Page".into()
            },
            JSXTagEnd,
            Semi,
        ]
    );
}

#[test]
fn issue_299_03() {
    assert_eq!(
        lex_tokens(
            ::Syntax::Es(::EsConfig {
                jsx: true,
                ..Default::default()
            }),
            "<Page num='\\\\'>ABC</Page>;"
        ),
        vec![
            Token::JSXTagStart,
            Token::JSXName {
                name: "Page".into()
            },
            Token::JSXName { name: "num".into() },
            tok!('='),
            Token::Str {
                value: "\\".into(),
                has_escape: true
            },
            Token::JSXTagEnd,
            JSXText { raw: "ABC".into() },
            JSXTagStart,
            tok!('/'),
            JSXName {
                name: "Page".into()
            },
            JSXTagEnd,
            Semi,
        ]
    );
}

#[test]
fn issue_316() {
    assert_eq!(
        lex_tokens(Default::default(), "'Hi\\r\\n..'"),
        vec![Token::Str {
            value: "Hi\r\n..".into(),
            has_escape: true
        }]
    );
}

#[test]
fn issue_401() {
    assert_eq!(
        lex_tokens(Default::default(), "'17' as const"),
        vec![
            Token::Str {
                value: "17".into(),
                has_escape: false
            },
            tok!("as"),
            tok!("const")
        ],
    );
}

#[bench]
fn lex_colors_js(b: &mut Bencher) {
    b.bytes = include_str!("../../colors.js").len() as _;

    b.iter(|| {
        let _ = with_lexer(
            Syntax::default(),
            include_str!("../../colors.js"),
            |lexer| {
                for _ in lexer {}
                Ok(())
            },
        );
    });
}
