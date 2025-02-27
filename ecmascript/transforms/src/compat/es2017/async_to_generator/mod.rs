use crate::{
    pass::Pass,
    util::{contains_this_expr, ExprFactory, StmtLike},
};
use ast::*;
use std::iter;
use swc_common::{Fold, FoldWith, Mark, Spanned, Visit, VisitWith, DUMMY_SP};

#[cfg(test)]
mod tests;

/// `@babel/plugin-transform-async-to-generator`
///
/// ## In
///
/// ```js
/// async function foo() {
///   await bar();
/// }
/// ```
///
/// ## Out
///
/// ```js
/// var _asyncToGenerator = function (fn) {
///   ...
/// };
/// var foo = _asyncToGenerator(function* () {
///   yield bar();
/// });
/// ```
pub fn async_to_generator() -> impl Pass {
    AsyncToGenerator
}

#[derive(Default, Clone)]
struct AsyncToGenerator;
struct Actual {
    extra_stmts: Vec<Stmt>,
}

impl<T> Fold<Vec<T>> for AsyncToGenerator
where
    T: StmtLike + VisitWith<AsyncVisitor> + FoldWith<Actual>,
    Vec<T>: FoldWith<Self>,
{
    fn fold(&mut self, stmts: Vec<T>) -> Vec<T> {
        if !contains_async(&stmts) {
            return stmts;
        }

        let stmts = stmts.fold_children(self);

        let mut buf = Vec::with_capacity(stmts.len());

        for stmt in stmts {
            if !contains_async(&stmt) {
                buf.push(stmt);
                continue;
            }

            let mut actual = Actual {
                extra_stmts: vec![],
            };
            let stmt = stmt.fold_with(&mut actual);

            buf.extend(actual.extra_stmts.into_iter().map(T::from_stmt));
            buf.push(stmt);
        }

        buf
    }
}

impl Fold<MethodProp> for Actual {
    fn fold(&mut self, prop: MethodProp) -> MethodProp {
        let prop = validate!(prop);
        let prop = prop.fold_children(self);

        if !prop.function.is_async {
            return prop;
        }
        let params = prop.function.params;

        let fn_ref = make_fn_ref(FnExpr {
            ident: None,
            function: Function {
                params: vec![],
                ..prop.function
            },
        });
        let fn_ref = Expr::Call(CallExpr {
            span: DUMMY_SP,
            callee: fn_ref.as_callee(),
            args: vec![],
            type_args: Default::default(),
        });

        MethodProp {
            function: Function {
                params,
                span: DUMMY_SP,
                is_async: false,
                is_generator: false,
                body: Some(BlockStmt {
                    span: DUMMY_SP,
                    stmts: vec![Stmt::Return(ReturnStmt {
                        span: DUMMY_SP,
                        arg: Some(box fn_ref),
                    })],
                }),
                decorators: Default::default(),
                return_type: Default::default(),
                type_params: Default::default(),
            },
            ..prop
        }
    }
}

/// Hoists super access
///
/// ## In
///
/// ```js
/// class Foo {
///     async foo () {
///         super.getter
///         super.setter = 1
///         super.method()
///     }
/// }
/// ```
///
/// ## OUt
///
/// ```js
/// class Foo {
///     var _super_getter = () => super.getter;
///     var _super_setter = (v) => super.setter = v;
///     var _super_method = (...args) => super.method(args);
///     foo () {
///         super.getter
///         super.setter = 1
///         super.method()
///     }
/// }
/// ```
struct MethodFolder {
    vars: Vec<VarDeclarator>,
}

impl MethodFolder {
    fn ident_for_super(&mut self, prop: &Expr) -> (Mark, Ident) {
        let mark = Mark::fresh(Mark::root());
        let prop_span = prop.span();
        let mut ident = match *prop {
            Expr::Ident(ref ident) => quote_ident!(prop_span, format!("_super_{}", ident.sym)),
            _ => quote_ident!(prop_span, "_super_method"),
        };
        ident.span = ident.span.apply_mark(mark);
        (mark, ident)
    }
}

impl Fold<Expr> for MethodFolder {
    fn fold(&mut self, expr: Expr) -> Expr {
        let expr = validate!(expr);
        // TODO(kdy): Cache (Reuse declaration for same property)

        match expr {
            // super.setter = 1
            Expr::Assign(AssignExpr {
                span,
                left:
                    PatOrExpr::Expr(box Expr::Member(MemberExpr {
                        span: m_span,
                        obj: ExprOrSuper::Super(super_token),
                        computed,
                        prop,
                    })),
                op,
                right,
            })
            | Expr::Assign(AssignExpr {
                span,
                left:
                    PatOrExpr::Pat(box Pat::Expr(box Expr::Member(MemberExpr {
                        span: m_span,
                        obj: ExprOrSuper::Super(super_token),
                        computed,
                        prop,
                    }))),
                op,
                right,
            }) => {
                let (mark, ident) = self.ident_for_super(&prop);
                let args_ident = quote_ident!(DUMMY_SP.apply_mark(mark), "_args");

                self.vars.push(VarDeclarator {
                    span: DUMMY_SP,
                    name: Pat::Ident(ident.clone()),
                    init: Some(box Expr::Arrow(ArrowExpr {
                        span: DUMMY_SP,
                        is_async: false,
                        is_generator: false,
                        params: vec![Pat::Ident(args_ident.clone())],
                        body: BlockStmtOrExpr::Expr(box Expr::Assign(AssignExpr {
                            span: DUMMY_SP,
                            left: PatOrExpr::Expr(
                                box MemberExpr {
                                    span: m_span,
                                    obj: ExprOrSuper::Super(super_token),
                                    computed,
                                    prop,
                                }
                                .into(),
                            ),
                            op,
                            right: box args_ident.into(),
                        })),
                        type_params: Default::default(),
                        return_type: Default::default(),
                    })),
                    definite: false,
                });

                Expr::Call(CallExpr {
                    span,
                    callee: ident.as_callee(),
                    args: vec![right.as_arg()],
                    type_args: Default::default(),
                })
            }

            // super.method()
            Expr::Call(CallExpr {
                span,
                callee:
                    ExprOrSuper::Expr(box Expr::Member(MemberExpr {
                        span: _,
                        obj: ExprOrSuper::Super(super_token),
                        prop,
                        computed,
                    })),
                args,
                type_args,
            }) => {
                let (mark, ident) = self.ident_for_super(&prop);
                let args_ident = quote_ident!(DUMMY_SP.apply_mark(mark), "_args");

                self.vars.push(VarDeclarator {
                    span: DUMMY_SP,
                    name: Pat::Ident(ident.clone()),
                    init: Some(box Expr::Arrow(ArrowExpr {
                        span: DUMMY_SP,
                        is_async: false,
                        is_generator: false,
                        params: vec![Pat::Rest(RestPat {
                            dot3_token: DUMMY_SP,
                            arg: box Pat::Ident(args_ident.clone()),
                            type_ann: Default::default(),
                        })],
                        body: BlockStmtOrExpr::Expr(box Expr::Call(CallExpr {
                            span: DUMMY_SP,
                            callee: MemberExpr {
                                span: DUMMY_SP,
                                obj: ExprOrSuper::Super(super_token),
                                computed,
                                prop,
                            }
                            .as_callee(),
                            args: vec![ExprOrSpread {
                                spread: Some(DUMMY_SP),
                                expr: box args_ident.clone().into(),
                            }],
                            type_args: Default::default(),
                        })),
                        type_params: Default::default(),
                        return_type: Default::default(),
                    })),
                    definite: false,
                });

                Expr::Call(CallExpr {
                    span,
                    callee: ident.as_callee(),
                    args,
                    type_args,
                })
            }
            // super.getter
            Expr::Member(MemberExpr {
                span,
                obj: ExprOrSuper::Super(super_token),
                prop,
                computed,
            }) => {
                let (_, ident) = self.ident_for_super(&prop);
                self.vars.push(VarDeclarator {
                    span: DUMMY_SP,
                    name: Pat::Ident(ident.clone()),
                    init: Some(box Expr::Arrow(ArrowExpr {
                        span: DUMMY_SP,
                        is_async: false,
                        is_generator: false,
                        params: vec![],
                        body: BlockStmtOrExpr::Expr(
                            box MemberExpr {
                                span: DUMMY_SP,
                                obj: ExprOrSuper::Super(super_token),
                                computed,
                                prop,
                            }
                            .into(),
                        ),
                        type_params: Default::default(),
                        return_type: Default::default(),
                    })),
                    definite: false,
                });

                Expr::Call(CallExpr {
                    span,
                    callee: ident.as_callee(),
                    args: vec![],
                    type_args: Default::default(),
                })
            }
            _ => expr.fold_children(self),
        }
    }
}

impl Fold<ClassMethod> for Actual {
    fn fold(&mut self, m: ClassMethod) -> ClassMethod {
        if m.function.body.is_none() {
            return m;
        }
        if m.kind != MethodKind::Method || !m.function.is_async {
            return m;
        }
        let params = m.function.params.clone();

        let mut folder = MethodFolder { vars: vec![] };
        let function = m.function.fold_children(&mut folder);
        let expr = make_fn_ref(FnExpr {
            ident: None,
            function,
        });

        let hoisted_super = if folder.vars.is_empty() {
            None
        } else {
            Some(Stmt::Decl(Decl::Var(VarDecl {
                span: DUMMY_SP,
                kind: VarDeclKind::Var,
                decls: folder.vars,
                declare: false,
            })))
        };

        ClassMethod {
            function: Function {
                span: m.span,
                is_async: false,
                is_generator: false,
                params,
                body: Some(BlockStmt {
                    span: DUMMY_SP,
                    stmts: hoisted_super
                        .into_iter()
                        .chain(iter::once(Stmt::Return(ReturnStmt {
                            span: DUMMY_SP,
                            arg: Some(box Expr::Call(CallExpr {
                                span: DUMMY_SP,
                                callee: expr.as_callee(),
                                args: vec![],
                                type_args: Default::default(),
                            })),
                        })))
                        .collect(),
                }),
                decorators: Default::default(),
                type_params: Default::default(),
                return_type: Default::default(),
            },
            ..m
        }
    }
}

impl Fold<Expr> for Actual {
    fn fold(&mut self, expr: Expr) -> Expr {
        let expr = validate!(expr);

        match expr {
            // Optimization for iife.
            Expr::Call(CallExpr {
                span,
                callee: ExprOrSuper::Expr(box Expr::Fn(fn_expr)),
                args,
                type_args,
            }) => {
                if !args.is_empty() || !fn_expr.function.is_async {
                    return Expr::Call(CallExpr {
                        span,
                        callee: ExprOrSuper::Expr(box Expr::Fn(fn_expr)),
                        args,
                        type_args,
                    });
                }

                return make_fn_ref(fn_expr);
            }
            _ => {}
        }

        let expr = expr.fold_children(self);

        match expr {
            Expr::Fn(
                expr @ FnExpr {
                    function:
                        Function {
                            is_async: true,
                            body: Some(..),
                            ..
                        },
                    ..
                },
            ) => {
                let function = self.fold_fn(expr.ident.clone(), expr.function, false);
                let body = Some(BlockStmt {
                    span: DUMMY_SP,
                    stmts: self
                        .extra_stmts
                        .drain(..)
                        .chain(function.body.unwrap().stmts)
                        .collect(),
                });

                Expr::Call(CallExpr {
                    span: DUMMY_SP,
                    callee: Expr::Fn(FnExpr {
                        ident: None,
                        function: Function { body, ..function },
                    })
                    .as_callee(),
                    args: vec![],
                    type_args: Default::default(),
                })
            }
            _ => expr,
        }
    }
}

impl Fold<FnDecl> for Actual {
    fn fold(&mut self, f: FnDecl) -> FnDecl {
        let f = f.fold_children(self);
        if !f.function.is_async {
            return f;
        }

        let function = self.fold_fn(Some(f.ident.clone()), f.function, true);
        FnDecl {
            ident: f.ident,
            function,
            declare: false,
        }
    }
}

impl Actual {
    #[inline(always)]
    fn fold_fn(&mut self, raw_ident: Option<Ident>, f: Function, is_decl: bool) -> Function {
        if f.body.is_none() {
            return f;
        }
        let span = f.span();
        let params = f.params.clone();
        let ident = raw_ident.clone().unwrap_or_else(|| quote_ident!("ref"));

        let real_fn_ident = private_ident!(ident.span, format!("_{}", ident.sym));
        let right = make_fn_ref(FnExpr {
            ident: None,
            function: f,
        });

        if is_decl {
            let real_fn = FnDecl {
                ident: real_fn_ident.clone(),
                declare: false,
                function: Function {
                    span: DUMMY_SP,
                    body: Some(BlockStmt {
                        span: DUMMY_SP,
                        stmts: vec![
                            Stmt::Expr(box Expr::Assign(AssignExpr {
                                span: DUMMY_SP,
                                left: PatOrExpr::Pat(box Pat::Ident(real_fn_ident.clone())),
                                op: op!("="),
                                right: box right,
                            })),
                            Stmt::Return(ReturnStmt {
                                span: DUMMY_SP,
                                arg: Some(box real_fn_ident.clone().apply(
                                    DUMMY_SP,
                                    box ThisExpr { span: DUMMY_SP }.into(),
                                    vec![quote_ident!("arguments").as_arg()],
                                )),
                            }),
                        ],
                    }),
                    params: vec![],
                    is_async: false,
                    is_generator: false,
                    decorators: Default::default(),
                    type_params: Default::default(),
                    return_type: Default::default(),
                },
            };
            self.extra_stmts.push(Stmt::Decl(Decl::Fn(real_fn)));
        } else {
            self.extra_stmts.push(Stmt::Decl(Decl::Var(VarDecl {
                span: DUMMY_SP,
                kind: VarDeclKind::Var,
                decls: vec![VarDeclarator {
                    span: DUMMY_SP,
                    name: Pat::Ident(real_fn_ident.clone()),
                    init: Some(box right),
                    definite: false,
                }],
                declare: false,
            })));
        }

        let apply = Stmt::Return(ReturnStmt {
            span: DUMMY_SP,
            arg: Some(box real_fn_ident.apply(
                DUMMY_SP,
                box Expr::This(ThisExpr { span: DUMMY_SP }),
                vec![quote_ident!("arguments").as_arg()],
            )),
        });
        Function {
            span,
            body: Some(BlockStmt {
                span: DUMMY_SP,
                stmts: if is_decl {
                    vec![apply]
                } else {
                    vec![Stmt::Return(ReturnStmt {
                        span: DUMMY_SP,
                        arg: Some(box Expr::Fn(FnExpr {
                            ident: raw_ident,
                            function: Function {
                                span: DUMMY_SP,
                                is_async: false,
                                is_generator: false,
                                params: vec![],
                                body: Some(BlockStmt {
                                    span: DUMMY_SP,
                                    stmts: vec![apply],
                                }),
                                decorators: Default::default(),
                                type_params: Default::default(),
                                return_type: Default::default(),
                            },
                        })),
                    })]
                },
            }),
            params: params.clone(),
            is_generator: false,
            is_async: false,
            decorators: Default::default(),
            return_type: Default::default(),
            type_params: Default::default(),
        }
    }
}

/// Creates
///
/// `_asyncToGenerator(function*() {})` from `async function() {}`;
fn make_fn_ref(mut expr: FnExpr) -> Expr {
    struct AwaitToYield;

    macro_rules! noop {
        ($T:path) => {
            impl Fold<$T> for AwaitToYield {
                /// Don't recurse into function.
                fn fold(&mut self, f: $T) -> $T {
                    f
                }
            }
        };
    }
    noop!(FnDecl);
    noop!(FnExpr);
    noop!(Constructor);
    noop!(ArrowExpr);

    impl Fold<Expr> for AwaitToYield {
        fn fold(&mut self, expr: Expr) -> Expr {
            let expr = expr.fold_children(self);

            match expr {
                Expr::Await(AwaitExpr { span, arg }) => Expr::Yield(YieldExpr {
                    span,
                    delegate: false,
                    arg: Some(arg),
                }),
                _ => expr,
            }
        }
    }

    expr.function.body = expr.function.body.fold_with(&mut AwaitToYield);

    assert!(expr.function.is_async);
    expr.function.is_async = false;
    expr.function.is_generator = true;

    let span = expr.span();

    let contains_this = contains_this_expr(&expr.function.body);
    let expr = if contains_this {
        Expr::Call(CallExpr {
            span: DUMMY_SP,
            callee: validate!(expr.member(quote_ident!("bind"))).as_callee(),
            args: vec![ThisExpr { span: DUMMY_SP }.as_arg()],
            type_args: Default::default(),
        })
    } else {
        Expr::Fn(expr)
    };

    Expr::Call(CallExpr {
        span,
        callee: helper!(async_to_generator, "asyncToGenerator"),
        args: vec![expr.as_arg()],
        type_args: Default::default(),
    })
}

fn contains_async<N>(node: &N) -> bool
where
    N: VisitWith<AsyncVisitor>,
{
    let mut v = AsyncVisitor { found: false };
    node.visit_with(&mut v);
    v.found
}

struct AsyncVisitor {
    found: bool,
}

impl Visit<Function> for AsyncVisitor {
    fn visit(&mut self, f: &Function) {
        if f.is_async {
            self.found = true;
        }
        f.visit_children(self);
    }
}
impl Visit<ArrowExpr> for AsyncVisitor {
    fn visit(&mut self, f: &ArrowExpr) {
        if f.is_async {
            self.found = true;
        }
        f.visit_children(self);
    }
}
