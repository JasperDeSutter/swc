use self::{
    constructor::{
        constructor_fn, make_possible_return_value, replace_this_in_constructor, ConstructorFolder,
        ReturningMode, SuperCallFinder, SuperFoldingMode, VarRenamer,
    },
    native::is_native,
    prop_name::HashKey,
    super_field::SuperFieldAccessFolder,
};
use crate::util::{
    alias_ident_for, default_constructor, prepend, prop_name_to_expr, ExprFactory, ModuleItemLike,
    StmtLike,
};
use ast::*;
use fxhash::FxBuildHasher;
use std::iter;
use swc_common::{Fold, FoldWith, Mark, Spanned, Visit, VisitWith, DUMMY_SP};

#[macro_use]
mod macros;
mod constructor;
mod native;
mod prop_name;
mod super_field;
#[cfg(test)]
mod tests;

type IndexMap<K, V> = indexmap::IndexMap<K, V, FxBuildHasher>;

/// `@babel/plugin-transform-classes`
///
/// # In
/// ```js
/// class Test {
///   constructor(name) {
///     this.name = name;
///   }
///
///   logger () {
///     console.log("Hello", this.name);
///   }
/// }
/// ```
///
/// # Out
/// ```js
/// var Test = function () {
///   function Test(name) {
///     _classCallCheck(this, Test);
///
///     this.name = name;
///   }
///
///   Test.prototype.logger = function logger() {
///     console.log("Hello", this.name);
///   };
///
///   return Test;
/// }();
/// ```
#[derive(Default, Clone, Copy)]
pub struct Classes;

struct Data {
    key_prop: Box<Prop>,
    method: Option<Box<Expr>>,
    set: Option<Box<Expr>>,
    get: Option<Box<Expr>>,
}

impl<T> Fold<Vec<T>> for Classes
where
    T: StmtLike + ModuleItemLike + FoldWith<Self>,
{
    fn fold(&mut self, stmts: Vec<T>) -> Vec<T> {
        let mut buf = Vec::with_capacity(stmts.len());

        for stmt in stmts {
            match T::try_into_stmt(stmt) {
                Err(node) => match node.try_into_module_decl() {
                    Ok(decl) => {
                        match decl {
                            ModuleDecl::ExportDefaultDecl(ExportDefaultDecl {
                                decl: DefaultDecl::Class(ClassExpr { ident, class }),
                                ..
                            }) => {
                                let ident = ident.unwrap_or_else(|| quote_ident!("_default"));

                                let decl = self.fold_class_as_var_decl(ident.clone(), class);
                                let decl = decl.fold_children(self);
                                buf.push(T::from_stmt(Stmt::Decl(Decl::Var(decl))));

                                buf.push(
                                    match T::try_from_module_decl(ModuleDecl::ExportNamed(
                                        NamedExport {
                                            span: DUMMY_SP,
                                            specifiers: vec![NamedExportSpecifier {
                                                span: DUMMY_SP,
                                                orig: ident,
                                                exported: Some(quote_ident!("default")),
                                            }
                                            .into()],
                                            src: None,
                                        },
                                    )) {
                                        Ok(t) => t,
                                        Err(..) => unreachable!(),
                                    },
                                );
                            }
                            ModuleDecl::ExportDecl(ExportDecl {
                                span,
                                decl:
                                    Decl::Class(ClassDecl {
                                        ident,
                                        declare: false,
                                        class,
                                    }),
                                ..
                            }) => {
                                let decl = self.fold_class_as_var_decl(ident, class);
                                let decl = decl.fold_children(self);
                                buf.push(
                                    match T::try_from_module_decl(ModuleDecl::ExportDecl(
                                        ExportDecl {
                                            span,
                                            decl: Decl::Var(decl),
                                        },
                                    )) {
                                        Ok(t) => t,
                                        Err(..) => unreachable!(),
                                    },
                                );
                            }
                            _ => {
                                buf.push(match T::try_from_module_decl(decl.fold_children(self)) {
                                    Ok(t) => t,
                                    Err(..) => unreachable!(),
                                })
                            }
                        };
                    }
                    Err(..) => unreachable!(),
                },
                Ok(stmt) => {
                    let stmt = stmt.fold_children(self);
                    buf.push(T::from_stmt(stmt));
                }
            }
        }

        buf
    }
}

impl Fold<Decl> for Classes {
    fn fold(&mut self, n: Decl) -> Decl {
        fn should_work(node: &Decl) -> bool {
            struct Visitor {
                found: bool,
            };
            impl Visit<Class> for Visitor {
                fn visit(&mut self, _: &Class) {
                    self.found = true
                }
            }
            let mut v = Visitor { found: false };
            node.visit_with(&mut v);
            v.found
        }
        // fast path
        if !should_work(&n) {
            return n;
        }

        let n = match n {
            Decl::Class(decl) => Decl::Var(self.fold_class_as_var_decl(decl.ident, decl.class)),
            _ => n,
        };

        n.fold_children(self)
    }
}

impl Fold<Expr> for Classes {
    fn fold(&mut self, n: Expr) -> Expr {
        let n = validate!(n);

        match n {
            Expr::Class(e) => self.fold_class(e.ident, e.class).fold_children(self),
            _ => n.fold_children(self),
        }
    }
}

impl Classes {
    fn fold_class_as_var_decl(&mut self, ident: Ident, class: Class) -> VarDecl {
        let span = class.span;
        let rhs = self.fold_class(Some(ident.clone()), class);

        VarDecl {
            span,
            kind: VarDeclKind::Let,
            decls: vec![VarDeclarator {
                span,
                init: Some(box rhs),
                // Foo in var Foo =
                name: ident.into(),
                definite: false,
            }],
            declare: false,
        }
    }

    /// Turns class expresion into iife.
    ///
    /// ```js
    /// class Foo {}
    /// ```
    ///
    /// ```js
    /// function() {
    ///   var Foo = function Foo(){
    ///   };
    /// }()
    /// ```
    fn fold_class(&mut self, class_name: Option<Ident>, class: Class) -> Expr {
        // Ident of the super class *inside* function.
        let super_ident = class
            .super_class
            .as_ref()
            .map(|e| alias_ident_for(e, "_super"));

        let (params, args) = if let Some(ref super_ident) = super_ident {
            let params = vec![Pat::Ident(super_ident.clone())];

            let super_class = class.super_class.clone().unwrap();
            let is_super_native = match *super_class {
                Expr::Ident(Ident { ref sym, .. }) => is_native(sym),
                _ => false,
            };
            if is_super_native {
                (
                    params,
                    vec![CallExpr {
                        span: DUMMY_SP,
                        callee: helper!(wrap_native_super, "wrapNativeSuper"),
                        args: vec![super_class.as_arg()],
                        type_args: Default::default(),
                    }
                    .as_arg()],
                )
            } else {
                (params, vec![super_class.as_arg()])
            }
        } else {
            (vec![], vec![])
        };

        let mut stmts = self.class_to_stmts(class_name, super_ident, class);

        if stmts.len() == 1 {
            //    class Foo {}
            //
            // should be
            //
            //    var Foo = function Foo() {
            //        _classCallCheck(this, Foo);
            //    };
            //
            // instead of
            //    var Foo = function(){
            //      function Foo() {
            //          _classCallCheck(this, Foo);
            //      }
            //
            //      return Foo;
            //    }();

            let stmt = stmts.pop().unwrap();
            match stmt {
                Stmt::Decl(Decl::Fn(FnDecl {
                    ident, function, ..
                })) => {
                    return Expr::Fn(FnExpr {
                        ident: Some(ident),
                        function,
                    });
                }
                _ => unreachable!(),
            }
        }
        let body = BlockStmt {
            span: DUMMY_SP,
            stmts,
        };

        Expr::Call(CallExpr {
            span: DUMMY_SP,
            callee: Expr::Fn(FnExpr {
                ident: None,
                function: Function {
                    span: DUMMY_SP,
                    is_async: false,
                    is_generator: false,
                    params,
                    body: Some(body),
                    decorators: Default::default(),
                    type_params: Default::default(),
                    return_type: Default::default(),
                },
            })
            .as_callee(),
            args,
            type_args: Default::default(),
        })
    }

    /// Returned `stmts` contains `return Foo`
    fn class_to_stmts(
        &mut self,
        class_name: Option<Ident>,
        super_class_ident: Option<Ident>,
        class: Class,
    ) -> Vec<Stmt> {
        let is_named = class_name.is_some();
        let class_name = class_name.unwrap_or_else(|| quote_ident!("_class"));
        let mut stmts = vec![];

        let mut priv_methods = vec![];
        let mut methods = vec![];
        let mut constructor = None;
        for member in class.body {
            match member {
                ClassMember::Constructor(c) => {
                    if constructor.is_some() {
                        unimplemented!("multiple constructor")
                    } else {
                        constructor = Some(c)
                    }
                }
                ClassMember::PrivateMethod(m) => priv_methods.push(m),
                ClassMember::Method(m) => methods.push(m),

                ClassMember::ClassProp(..) => {
                    unreachable!("classes pass: property\nclass_properties pass should remove this")
                }
                ClassMember::PrivateProp(..) => unreachable!(
                    "classes pass: private property\nclass_properties pass should remove this"
                ),
                ClassMember::TsIndexSignature(s) => {
                    unimplemented!("typescript index signature {:?}", s)
                }
            }
        }

        if let Some(ref super_class_ident) = super_class_ident {
            // inject helper methods

            stmts.push(Stmt::Expr(box Expr::Call(CallExpr {
                span: DUMMY_SP,
                callee: helper!(inherits, "inherits"),
                args: vec![
                    class_name.clone().as_arg(),
                    super_class_ident.clone().as_arg(),
                ],
                type_args: Default::default(),
            })));
        }

        // Marker for `_this`
        let mark = Mark::fresh(Mark::root());

        {
            // Process constructor

            let mut constructor =
                constructor.unwrap_or_else(|| default_constructor(super_class_ident.is_some()));

            // Rename variables to avoid conflicting with class name
            constructor.body = constructor.body.fold_with(&mut VarRenamer {
                mark: Mark::fresh(Mark::root()),
                class_name: &class_name.sym,
            });

            // Black magic to detect injected constructor.
            let is_constructor_default = constructor.span.is_dummy();
            if is_constructor_default {
                constructor.params = vec![];
            }

            let mut insert_this = false;

            if super_class_ident.is_some() {
                let (c, inserted_this) = replace_this_in_constructor(mark, constructor);
                constructor = c;
                insert_this |= inserted_this;
            }

            // inject _classCallCheck(this, Bar);
            inject_class_call_check(&mut constructor, class_name.clone());
            let mut body = constructor.body.unwrap().stmts;
            // should we insert `var _this`?

            let is_always_initialized = is_always_initialized(&body);

            // We should handle branching
            if !is_always_initialized {
                insert_this = true;
            }

            // inject possibleReturnCheck
            let mode = if insert_this {
                Some(SuperFoldingMode::Assign)
            } else {
                SuperCallFinder::find(&body)
            };

            if super_class_ident.is_some() {
                let this = quote_ident!(DUMMY_SP.apply_mark(mark), "_this");

                // We should fold body instead of constructor itself.
                // Handle `super()`
                body = body.fold_with(&mut ConstructorFolder {
                    is_constructor_default,
                    class_name: &class_name,
                    mode,
                    mark,
                    ignore_return: false,
                });

                insert_this |= (mode == None && !is_always_initialized)
                    || mode == Some(SuperFoldingMode::Assign);

                if insert_this {
                    prepend(
                        &mut body,
                        Stmt::Decl(Decl::Var(VarDecl {
                            span: DUMMY_SP,
                            declare: false,
                            kind: VarDeclKind::Var,
                            decls: vec![VarDeclarator {
                                span: DUMMY_SP,
                                name: Pat::Ident(this.clone()),
                                init: None,
                                definite: false,
                            }],
                        })),
                    );
                }

                let is_last_return = match body.last() {
                    Some(Stmt::Return(..)) => true,
                    _ => false,
                };
                if !is_last_return {
                    if is_always_initialized {
                        body.push(Stmt::Return(ReturnStmt {
                            span: DUMMY_SP,
                            arg: Some(box Expr::Ident(this)),
                        }));
                    } else {
                        let possible_return_value =
                            box make_possible_return_value(ReturningMode::Returning {
                                mark,
                                arg: None,
                            });
                        body.push(Stmt::Return(ReturnStmt {
                            span: DUMMY_SP,
                            arg: Some(possible_return_value),
                        }));
                    }
                }
            }

            let is_this_declared = (insert_this && super_class_ident.is_some())
                || (mode == Some(SuperFoldingMode::Var));

            // Handle `super.XX`
            body = self.handle_super_access(
                &class_name,
                body,
                if is_this_declared { Some(mark) } else { None },
            );

            stmts.push(Stmt::Decl(Decl::Fn(FnDecl {
                ident: class_name.clone(),
                function: constructor_fn(Constructor {
                    body: Some(BlockStmt {
                        span: DUMMY_SP,
                        stmts: body,
                    }),
                    ..constructor
                }),
                declare: false,
            })));
        }

        // convert class methods
        // stmts.extend(self.fold_class_methods(class_name.clone(), priv_methods));
        stmts.extend(self.fold_class_methods(class_name.clone(), methods));

        if is_named && stmts.len() == 1 {
            return stmts;
        }

        // `return Foo`
        stmts.push(Stmt::Return(ReturnStmt {
            span: DUMMY_SP,
            arg: Some(box Expr::Ident(class_name)),
        }));

        stmts
    }

    ///
    /// - `this_mark`: `Some(mark)` if we injected `var _this;`; otherwise
    ///   `None`
    fn handle_super_access(
        &mut self,
        class_name: &Ident,
        body: Vec<Stmt>,
        this_mark: Option<Mark>,
    ) -> Vec<Stmt> {
        let mut vars = vec![];
        let mut folder = SuperFieldAccessFolder {
            class_name,
            vars: &mut vars,
            constructor_this_mark: this_mark,
            // constructor cannot be static
            is_static: false,
            folding_constructor: true,
            in_nested_scope: false,
            in_injected_define_property_call: false,
            this_alias_mark: None,
        };

        let mut body = body.fold_with(&mut folder);

        if let Some(mark) = folder.this_alias_mark {
            prepend(
                &mut body,
                Stmt::Decl(Decl::Var(VarDecl {
                    span: DUMMY_SP,
                    declare: false,
                    kind: VarDeclKind::Var,
                    decls: vec![VarDeclarator {
                        span: DUMMY_SP,
                        name: Pat::Ident(quote_ident!(DUMMY_SP.apply_mark(mark), "_this")),
                        init: Some(box Expr::This(ThisExpr { span: DUMMY_SP })),
                        definite: false,
                    }],
                })),
            );
        }

        if !vars.is_empty() {
            prepend(
                &mut body,
                Stmt::Decl(Decl::Var(VarDecl {
                    span: DUMMY_SP,
                    kind: VarDeclKind::Var,
                    declare: false,
                    decls: vars,
                })),
            );
        }
        body
    }

    fn fold_class_methods(&mut self, class_name: Ident, methods: Vec<ClassMethod>) -> Vec<Stmt> {
        if methods.is_empty() {
            return vec![];
        }

        /// { key: "prop" }
        fn mk_key_prop(key: &PropName) -> Prop {
            Prop::KeyValue(KeyValueProp {
                key: PropName::Ident(quote_ident!(key.span(), "key")),
                value: match *key {
                    PropName::Ident(ref i) => {
                        box Expr::Lit(Lit::Str(quote_str!(i.span, i.sym.clone())))
                    }
                    PropName::Str(ref s) => box Expr::Lit(Lit::Str(s.clone())),
                    PropName::Num(n) => box Expr::Lit(Lit::Num(n)),
                    PropName::Computed(ref expr) => expr.clone(),
                },
            })
        }

        fn mk_arg_obj_for_create_class(props: IndexMap<HashKey, Data>) -> ExprOrSpread {
            if props.is_empty() {
                return quote_expr!(DUMMY_SP, null).as_arg();
            }
            Expr::Array(ArrayLit {
                span: DUMMY_SP,
                elems: props
                    .into_iter()
                    .map(|(_, data)| {
                        let mut props = vec![PropOrSpread::Prop(data.key_prop)];

                        macro_rules! add {
                            ($field:expr, $kind:expr, $s:literal) => {{
                                if let Some(value) = $field {
                                    props.push(PropOrSpread::Prop(box Prop::KeyValue(
                                        KeyValueProp {
                                            key: PropName::Ident(quote_ident!($s)),
                                            value,
                                        },
                                    )));
                                }
                            }};
                        }

                        add!(data.get, MethodKind::Getter, "get");
                        add!(data.set, MethodKind::Setter, "set");
                        add!(data.method, MethodKind::Method, "value");

                        ObjectLit {
                            span: DUMMY_SP,
                            props,
                        }
                        .as_arg()
                    })
                    .map(Some)
                    .collect(),
            })
            .as_arg()
        }

        /// _createClass(Foo, [{}], [{}]);
        fn mk_create_class_call(
            class_name: Ident,
            methods: ExprOrSpread,
            static_methods: Option<ExprOrSpread>,
        ) -> Stmt {
            Stmt::Expr(box Expr::Call(CallExpr {
                span: DUMMY_SP,
                callee: helper!(create_class, "createClass"),
                args: iter::once(class_name.as_arg())
                    .chain(iter::once(methods))
                    .chain(static_methods)
                    .collect(),
                type_args: Default::default(),
            }))
        }

        let (mut props, mut static_props) = (IndexMap::default(), IndexMap::default());

        for m in methods {
            let key = HashKey::from(&m.key);
            let key_prop = box mk_key_prop(&m.key);
            let computed = match m.key {
                PropName::Computed(..) => true,
                _ => false,
            };
            let prop_name = prop_name_to_expr(m.key);

            let append_to: &mut IndexMap<_, _> = if m.is_static {
                &mut static_props
            } else {
                &mut props
            };

            let mut vars = vec![];
            let mut folder = SuperFieldAccessFolder {
                class_name: &class_name,
                vars: &mut vars,
                constructor_this_mark: None,
                is_static: m.is_static,
                folding_constructor: false,
                in_nested_scope: false,
                in_injected_define_property_call: false,
                this_alias_mark: None,
            };
            let mut function = m.function.fold_with(&mut folder);

            if let Some(mark) = folder.this_alias_mark {
                prepend(
                    &mut function.body.as_mut().unwrap().stmts,
                    Stmt::Decl(Decl::Var(VarDecl {
                        span: DUMMY_SP,
                        declare: false,
                        kind: VarDeclKind::Var,
                        decls: vec![VarDeclarator {
                            span: DUMMY_SP,
                            name: Pat::Ident(quote_ident!(DUMMY_SP.apply_mark(mark), "_this")),
                            init: Some(box Expr::This(ThisExpr { span: DUMMY_SP })),
                            definite: false,
                        }],
                    })),
                );
            }

            if !vars.is_empty() {
                prepend(
                    &mut function.body.as_mut().unwrap().stmts,
                    Stmt::Decl(Decl::Var(VarDecl {
                        span: DUMMY_SP,
                        kind: VarDeclKind::Var,
                        declare: false,
                        decls: vars,
                    })),
                );
            }

            let value = box Expr::Fn(FnExpr {
                ident: if m.kind == MethodKind::Method && !computed {
                    match prop_name {
                        Expr::Ident(ident) => Some(ident),
                        Expr::Lit(Lit::Str(Str { span, value, .. })) => {
                            Some(Ident::new(value, span))
                        }
                        _ => None,
                    }
                } else {
                    None
                },
                function,
            });

            let data = append_to.entry(key).or_insert_with(|| Data {
                key_prop,
                get: None,
                set: None,
                method: None,
            });
            match m.kind {
                MethodKind::Getter => data.get = Some(value),
                MethodKind::Setter => data.set = Some(value),
                MethodKind::Method => data.method = Some(value),
            }
        }

        if props.is_empty() && static_props.is_empty() {
            return vec![];
        }
        vec![mk_create_class_call(
            class_name,
            mk_arg_obj_for_create_class(props),
            if static_props.is_empty() {
                None
            } else {
                Some(mk_arg_obj_for_create_class(static_props))
            },
        )]
    }
}

/// Creates
///
/// ```js
/// Child.__proto__ || Object.getPrototypeOf(Child)
/// ```
fn get_prototype_of(obj: &Expr) -> Expr {
    Expr::Call(CallExpr {
        span: DUMMY_SP,
        callee: helper!(get_prototype_of, "getPrototypeOf"),
        args: vec![obj.clone().as_arg()],
        type_args: Default::default(),
    })
}

fn inject_class_call_check(c: &mut Constructor, name: Ident) {
    let class_call_check = Stmt::Expr(box Expr::Call(CallExpr {
        span: DUMMY_SP,
        callee: helper!(class_call_check, "classCallCheck"),
        args: vec![
            Expr::This(ThisExpr { span: DUMMY_SP }).as_arg(),
            Expr::Ident(name).as_arg(),
        ],
        type_args: Default::default(),
    }));

    prepend(&mut c.body.as_mut().unwrap().stmts, class_call_check)
}

/// Returns true if no `super` is used before `super()` call.
fn is_always_initialized(body: &[Stmt]) -> bool {
    struct SuperFinder {
        found: bool,
    }

    impl Visit<ExprOrSuper> for SuperFinder {
        fn visit(&mut self, node: &ExprOrSuper) {
            match *node {
                ExprOrSuper::Super(..) => self.found = true,
                _ => node.visit_children(self),
            }
        }
    }

    let pos = match body.iter().position(|s| match s {
        Stmt::Expr(box Expr::Call(CallExpr {
            callee: ExprOrSuper::Super(..),
            ..
        })) => true,
        _ => false,
    }) {
        Some(pos) => pos,
        _ => return false,
    };

    let mut v = SuperFinder { found: false };
    let body = &body[..pos];

    body.visit_with(&mut v);

    if v.found {
        return false;
    }

    true
}
