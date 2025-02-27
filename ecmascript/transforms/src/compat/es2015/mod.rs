pub use self::{
    arrow::arrow, block_scoped_fn::BlockScopedFns, block_scoping::block_scoping, classes::Classes,
    computed_props::computed_properties, destructuring::destructuring,
    duplicate_keys::duplicate_keys, for_of::for_of, function_name::function_name,
    instanceof::InstanceOf, parameters::parameters, shorthand_property::Shorthand, spread::spread,
    sticky_regex::StickyRegex, template_literal::TemplateLiteral, typeof_symbol::TypeOfSymbol,
};
use crate::pass::Pass;
use ast::{Expr, Module};

mod arrow;
mod block_scoped_fn;
mod block_scoping;
mod classes;
mod computed_props;
mod destructuring;
mod duplicate_keys;
mod for_of;
mod function_name;
mod instanceof;
mod parameters;
mod shorthand_property;
mod spread;
mod sticky_regex;
mod template_literal;
mod typeof_symbol;

fn exprs() -> impl Pass {
    chain_at!(
        Expr,
        arrow(),
        duplicate_keys(),
        StickyRegex,
        InstanceOf,
        TypeOfSymbol,
        Shorthand,
    )
}

/// Compiles es2015 to es5.
pub fn es2015() -> impl Pass {
    chain_at!(
        Module,
        BlockScopedFns,
        TemplateLiteral::default(),
        Classes,
        spread(),
        function_name(),
        exprs(),
        parameters(),
        for_of(),
        computed_properties(),
        destructuring(),
        block_scoping(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resolver;

    test!(
        ::swc_ecma_parser::Syntax::default(),
        |_| es2015(),
        issue_169,
        r#"
export class Foo {
	func(a, b = Date.now()) {
		return {a};
	}
}
"#,
        r#"
export var Foo = function() {
    function Foo() {
        _classCallCheck(this, Foo);
    }

    _createClass(Foo, [{
            key: 'func',
            value: function func(a, param) {
                var b = param === void 0 ? Date.now() : param;
                return {
                    a: a
                };
            }
        }]);
    return Foo;
}();
"#
    );

    test!(
        ::swc_ecma_parser::Syntax::default(),
        |_| es2015(),
        issue_189,
        r#"
class HomePage extends React.Component {}
"#,
        r#"
var HomePage = function(_Component) {
    _inherits(HomePage, _Component);
    function HomePage() {
        _classCallCheck(this, HomePage);
        return _possibleConstructorReturn(this, _getPrototypeOf(HomePage).apply(this, arguments));
    }
    return HomePage;
}(React.Component);
"#
    );

    test!(
        ::swc_ecma_parser::Syntax::default(),
        |_| es2015(),
        issue_227,
        "export default function fn1(...args) {
  fn2(...args);
}",
        "
export default function fn1() {
    for(var _len = arguments.length, args = new Array(_len), _key = 0; _key < _len; _key++){
        args[_key] = arguments[_key];
    }
    fn2.apply(void 0, args);
}
"
    );

    test!(
        ::swc_ecma_parser::Syntax::default(),
        |_| chain!(BlockScopedFns, resolver(),),
        issue_271,
        "
function foo(scope) {
    scope.startOperation = startOperation;

    function startOperation(operation) {
        scope.agentOperation = operation;
    }
}
",
        "
function foo(scope) {
    let startOperation = function startOperation(operation) {
        scope.agentOperation = operation;
    };
    scope.startOperation = startOperation;
}
"
    );

    //     test!(
    //         ::swc_ecma_parser::Syntax::default(),
    //         |_| chain!(
    //             resolver(),
    //             class_properties(),
    //             // Optional::new(compat::es2018(), target <= JscTarget::Es2018),
    //             // Optional::new(compat::es2017(), target <= JscTarget::Es2017),
    //             // Optional::new(compat::es2016(), target <= JscTarget::Es2016),
    //             // Optional::new(compat::es2015(), target <= JscTarget::Es2015),
    //             // Optional::new(compat::es3(), target <= JscTarget::Es3),
    //             hygiene(),
    //             fixer(),
    //         ),
    //         issue_405,
    //         "function Quadtree$1(x, y, x0, y0, x1, y1) {
    //     this._x = x;
    //     this._y = y;
    //     this._x0 = x0;
    //     this._y0 = y0;
    //     this._x1 = x1;
    //     this._y1 = y1;
    //     this._root = undefined;
    //   }
    //   ",
    //         ""
    //     );

    test!(
        ::swc_ecma_parser::Syntax::default(),
        |_| es2015(),
        issue_413,
        r#"
export const getBadgeBorderRadius = (text, color) => {
  return (text && style) || {}
}"#,
        r#"
export var getBadgeBorderRadius = function(text, color) {
    return text && style || {
    };
};
"#
    );

    test!(
        ::swc_ecma_parser::Syntax::default(),
        |_| es2015(),
        issue_400_1,
        "class A {
    constructor() {
        this.a_num = 10;
    }

    print() {
        expect(this.a_num).toBe(10);
    }
}

class B extends A {
    constructor(num) {
        super();
        this.b_num = num;
    }

    print() {
        expect(this.b_num).toBe(20);
        super.print();
    }
}
",
        "var A = function() {
    function A() {
        _classCallCheck(this, A);
        this.a_num = 10;
    }
    _createClass(A, [{
            key: 'print',
            value: function print() {
                expect(this.a_num).toBe(10);
            }
        }]);
    return A;
}();
var B = function(_A) {
    _inherits(B, _A);
    function B(num) {
        var _this;
        _classCallCheck(this, B);
        _this = _possibleConstructorReturn(this, _getPrototypeOf(B).call(this));
        _this.b_num = num;
        return _this;
    }
    _createClass(B, [{
            key: 'print',
            value: function print() {
                expect(this.b_num).toBe(20);
                _get(_getPrototypeOf(B.prototype), 'print', this).call(this);
            }
        }]);
    return B;
}(A);"
    );

    test_exec!(
        ::swc_ecma_parser::Syntax::default(),
        |_| es2015(),
        issue_400_2,
        "class A {
    constructor() {
        this.a_num = 10;
    }

    print() {
        expect(this.a_num).toBe(10);
    }
}

class B extends A {
    constructor(num) {
        super();
        this.b_num = num;
    }

    print() {
        expect(this.b_num).toBe(20);
        super.print();
    }
}

return new B(20).print()"
    );
}
