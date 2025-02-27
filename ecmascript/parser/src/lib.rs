//! es2019 parser
//!
//! # Features
//!
//! ## Heavily tested
//!
//! Passes almost all tests from [tc39/test262][].
//!
//! ## Error reporting
//!
//! ```sh
//! error: 'implements', 'interface', 'let', 'package', 'private', 'protected',  'public', 'static', or 'yield' cannot be used as an identifier in strict mode
//!  --> invalid.js:3:10
//!   |
//! 3 | function yield() {
//!   |          ^^^^^
//! ```
//!
//! # Example (lexer)
//!
//! See `lexer.rs` in examples directory.
//!
//! # Example (parser)
//!
//! ```
//! #[macro_use]
//! extern crate swc_common;
//! extern crate swc_ecma_parser;
//! use std::sync::Arc;
//! use swc_common::{
//!     errors::{ColorConfig, Handler},
//!     FileName, FilePathMapping, SourceMap,
//! };
//! use swc_ecma_parser::{lexer::Lexer, Parser, Session, SourceFileInput, Syntax};
//!
//! fn main() {
//!     swc_common::GLOBALS.set(&swc_common::Globals::new(), || {
//!         let cm: Arc<SourceMap> = Default::default();
//!         let handler =
//!             Handler::with_tty_emitter(ColorConfig::Auto, true, false, Some(cm.clone()));
//!
//!         let session = Session { handler: &handler };
//!
//!         // Real usage
//!         // let fm = cm
//!         //     .load_file(Path::new("test.js"))
//!         //     .expect("failed to load test.js");
//!
//!
//!         let fm = cm.new_source_file(
//!             FileName::Custom("test.js".into()),
//!             "function foo() {}".into(),
//!         );
//!         let lexer = Lexer::new(
//!             session,
//!             Syntax::Es(Default::default()),
//!             SourceFileInput::from(&*fm),
//!             None,
//!         );
//!
//!         let mut parser = Parser::new_from(session, lexer);
//!
//!
//!         let _module = parser
//!             .parse_module()
//!             .map_err(|mut e| {
//!                 e.emit();
//!                 ()
//!             })
//!             .expect("failed to parser module");
//!     });
//! }
//! ```
//!
//!
//! [tc39/test262]:https://github.com/tc39/test262

#![cfg_attr(any(test, feature = "fold"), feature(specialization))]
#![cfg_attr(test, feature(box_syntax))]
#![cfg_attr(test, feature(test))]
#![deny(unreachable_patterns)]
#![deny(unsafe_code)]

extern crate either;
#[macro_use]
extern crate smallvec;
extern crate swc_ecma_parser_macros as parser_macros;
#[macro_use]
extern crate log;
#[macro_use(js_word)]
extern crate swc_atoms;
extern crate enum_kind;
extern crate regex;
extern crate serde;
extern crate swc_common;
#[macro_use]
extern crate lazy_static;
extern crate swc_ecma_ast as ast;
#[macro_use]
#[cfg(test)]
extern crate testing;
#[cfg(test)]
extern crate env_logger;
#[cfg(test)]
extern crate test;
extern crate unicode_xid;
pub use self::{
    lexer::input::{Input, SourceFileInput},
    parser::*,
};
use serde::{Deserialize, Serialize};
use swc_common::errors::Handler;

#[macro_use]
mod macros;
mod error;
pub mod lexer;
mod parser;
mod token;

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(tag = "syntax")]
pub enum Syntax {
    /// Standard
    #[serde(rename = "ecmascript")]
    Es(EsConfig),
    #[serde(rename = "typescript")]
    Typescript(TsConfig),
}

impl Default for Syntax {
    fn default() -> Self {
        Syntax::Es(Default::default())
    }
}

impl Syntax {
    /// Should we pare jsx?
    pub fn jsx(self) -> bool {
        match self {
            Syntax::Es(EsConfig { jsx: true, .. })
            | Syntax::Typescript(TsConfig { tsx: true, .. }) => true,
            _ => false,
        }
    }

    pub fn dynamic_import(self) -> bool {
        match self {
            Syntax::Es(EsConfig {
                dynamic_import: true,
                ..
            })
            | Syntax::Typescript(TsConfig {
                dynamic_import: true,
                ..
            }) => true,
            _ => false,
        }
    }

    pub fn fn_bind(self) -> bool {
        match self {
            Syntax::Es(EsConfig { fn_bind: true, .. }) => true,
            _ => false,
        }
    }

    pub fn num_sep(self) -> bool {
        match self {
            Syntax::Es(EsConfig { num_sep: true, .. }) => true,
            _ => false,
        }
    }

    pub fn decorators(self) -> bool {
        match self {
            Syntax::Es(EsConfig {
                decorators: true, ..
            })
            | Syntax::Typescript(TsConfig {
                decorators: true, ..
            }) => true,
            _ => false,
        }
    }

    pub fn class_private_methods(self) -> bool {
        match self {
            Syntax::Es(EsConfig {
                class_private_methods: true,
                ..
            }) => true,
            _ => false,
        }
    }

    pub fn class_private_props(self) -> bool {
        match self {
            Syntax::Es(EsConfig {
                class_private_props: true,
                ..
            }) => true,
            _ => false,
        }
    }

    pub fn class_props(self) -> bool {
        if self.typescript() {
            return true;
        }
        match self {
            Syntax::Es(EsConfig {
                class_props: true, ..
            }) => true,
            _ => false,
        }
    }

    pub fn decorators_before_export(self) -> bool {
        match self {
            Syntax::Es(EsConfig {
                decorators_before_export: true,
                ..
            })
            | Syntax::Typescript(..) => true,
            _ => false,
        }
    }

    /// Should we pare typescript?
    pub fn typescript(self) -> bool {
        match self {
            Syntax::Typescript(..) => true,
            _ => false,
        }
    }

    pub fn export_default_from(self) -> bool {
        match self {
            Syntax::Es(EsConfig {
                export_default_from: true,
                ..
            }) => true,
            _ => false,
        }
    }

    pub fn export_namespace_from(self) -> bool {
        match self {
            Syntax::Es(EsConfig {
                export_namespace_from: true,
                ..
            }) => true,
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TsConfig {
    #[serde(default)]
    pub tsx: bool,

    #[serde(default)]
    pub decorators: bool,

    #[serde(default)]
    pub dynamic_import: bool,
}

#[derive(Clone, Copy, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct EsConfig {
    #[serde(default)]
    pub jsx: bool,
    /// Support numeric separator.
    #[serde(rename = "numericSeparator")]
    #[serde(default)]
    pub num_sep: bool,

    #[serde(rename = "classPrivateProperty")]
    #[serde(default)]
    pub class_private_props: bool,

    #[serde(rename = "privateMethod")]
    #[serde(default)]
    pub class_private_methods: bool,

    #[serde(rename = "classProperty")]
    #[serde(default)]
    pub class_props: bool,

    /// Support function bind expression.
    #[serde(rename = "functionBind")]
    #[serde(default)]
    pub fn_bind: bool,

    /// Enable decorators.
    #[serde(default)]
    pub decorators: bool,

    /// babel: `decorators.decoratorsBeforeExport`
    ///
    /// Effective only if `decorator` is true.
    #[serde(rename = "decoratorsBeforeExport")]
    #[serde(default)]
    pub decorators_before_export: bool,

    #[serde(default)]
    pub export_default_from: bool,

    #[serde(default)]
    pub export_namespace_from: bool,

    #[serde(default)]
    pub dynamic_import: bool,
}

/// Syntactic context.
#[derive(Debug, Clone, Copy, Default)]
pub struct Context {
    /// Is in module code?
    module: bool,
    strict: bool,
    include_in_expr: bool,
    /// If true, await expression is parsed, and "await" is treated as a
    /// keyword.
    in_async: bool,
    /// If true, yield expression is parsed, and "yield" is treated as a
    /// keyword.
    in_generator: bool,

    in_type: bool,
    /// Typescript extension.
    in_declare: bool,

    /// If true, `:` should not be treated as a type annotation.
    in_cond_expr: bool,

    in_function: bool,

    in_parameters: bool,

    in_method: bool,
    in_class_prop: bool,

    in_property_name: bool,

    in_forced_jsx_context: bool,
}

#[derive(Clone, Copy)]
pub struct Session<'a> {
    pub handler: &'a Handler,
}

#[cfg(test)]
fn with_test_sess<F, Ret>(src: &'static str, f: F) -> Result<Ret, ::testing::StdErr>
where
    F: FnOnce(Session, SourceFileInput) -> Result<Ret, ()>,
{
    use swc_common::FileName;

    ::testing::run_test(false, |cm, handler| {
        let fm = cm.new_source_file(FileName::Real("testing".into()), src.into());

        f(Session { handler: &handler }, (&*fm).into())
    })
}
