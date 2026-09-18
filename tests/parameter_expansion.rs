use bash_condexp::{
    Env, EvalError, Evaluator, MapEnv, ParseError, ParseOptions, StdFs, parse, parse_with_options,
};

fn evaluate(input: &str, env: &mut MapEnv) -> Result<bool, EvalError> {
    let expression = parse(input).unwrap_or_else(|error| panic!("parse {input:?}: {error}"));
    Evaluator::new(env, &StdFs).eval(&expression)
}

fn check(input: &str, env: &mut MapEnv) {
    assert!(
        evaluate(input, env).unwrap_or_else(|error| panic!("eval {input:?}: {error}")),
        "expression was false: {input}"
    );
}

fn evaluate_with_punctuation_vars(input: &str, env: &mut MapEnv) -> Result<bool, EvalError> {
    let options = ParseOptions::default().punctuation_variables(true);
    let expression = parse_with_options(input, options)
        .unwrap_or_else(|error| panic!("parse {input:?}: {error}"));
    Evaluator::new(env, &StdFs).eval(&expression)
}

fn check_with_punctuation_vars(input: &str, env: &mut MapEnv) {
    assert!(
        evaluate_with_punctuation_vars(input, env)
            .unwrap_or_else(|error| panic!("eval {input:?}: {error}")),
        "expression was false: {input}"
    );
}

#[test]
fn prefix_and_suffix_removal() {
    let mut env = MapEnv::new()
        .with_var("path", "a/b/c")
        .with_var("/", "._resource");
    check("${path#*/} == b/c", &mut env);
    check("${path##*/} == c", &mut env);
    check("${path%/*} == a/b", &mut env);
    check("${path%%/*} == a", &mut env);
    check_with_punctuation_vars("${/#._} == resource", &mut env);
}

#[test]
fn replacement_modes_and_matched_text() {
    let mut env = MapEnv::new().with_var("value", "abcabc").with_var("/", "b");
    check("${value/a/X} == Xbcabc", &mut env);
    check("${value//a/X} == XbcXbc", &mut env);
    check("${value/#a/X} == Xbcabc", &mut env);
    check("${value/%c/X} == abcabX", &mut env);
    check("${value/#/X} == Xabcabc", &mut env);
    check("${value/%/X} == abcabcX", &mut env);
    env.vars.insert("empty".into(), String::new());
    check("${value/$empty/X} == abcabc", &mut env);
    check("${value//?/[&]} == '[a][b][c][a][b][c]'", &mut env);
    check(r"${value//?/\&} == '&&&&&&'", &mut env);
    check_with_punctuation_vars("${value/${/}/X} == aXcabc", &mut env);

    env.options.insert("patsub_replacement".into(), false);
    check("${value/a/[&]} == '[&]bcabc'", &mut env);
}

#[test]
fn substring_length_case_and_nested_expansion() {
    let mut env = MapEnv::new()
        .with_var("value", "abcdef")
        .with_var("offset", "1")
        .with_var("fallback", "default")
        .with_var("letters", "abCab");
    check("${#value} -eq 6", &mut env);
    check("${value:2:3} == cde", &mut env);
    check("${value: -2} == ef", &mut env);
    check("${value:offset+1:2} == cd", &mut env);
    check("${value:offset=2:1} == c", &mut env);
    assert_eq!(env.var("offset"), Some("2"));
    assert!(matches!(
        evaluate("${value:6:-1}", &mut env),
        Err(EvalError::InvalidArith(_))
    ));
    check("${value:7:-1} == ''", &mut env);
    check("${letters^} == AbCab", &mut env);
    check("${letters^^@(a|b)} == ABCAB", &mut env);
    check("${letters,,C} == abcab", &mut env);
    env.vars.insert("empty".into(), String::new());
    check("${letters^$empty} == AbCab", &mut env);
    check("${letters^\"$empty\"} == abCab", &mut env);
    check("${missing:-${fallback}} == default", &mut env);

    env.vars.insert("value".into(), "é🙂".into());
    check("${#value} -eq 2", &mut env);
    check("${value/é/ø} == ø🙂", &mut env);
}

#[test]
fn default_and_alternate_test_unset_separately_from_null() {
    let mut env = MapEnv::new().with_var("empty", "").with_var("set", "value");
    check("${missing-fallback} == fallback", &mut env);
    check("${empty-fallback} == ''", &mut env);
    check("${empty:-fallback} == fallback", &mut env);
    check("${set+alternate} == alternate", &mut env);
    check("${empty+alternate} == alternate", &mut env);
    check("${empty:+alternate} == ''", &mut env);
    check("${missing+alternate} == ''", &mut env);

    // Quotes in a selected word remain meaningful when the complete expansion
    // is itself used as a conditional pattern.
    check("foo != ${missing-'*'}", &mut env);
    check("foo == ${missing-*}", &mut env);

    env.vars.insert("offset".into(), "1".into());
    check("${set:-${set:offset=9}} == value", &mut env);
    assert_eq!(env.var("offset"), Some("1"));
}

#[test]
fn extglob_and_nocasematch_follow_context_specific_rules() {
    let mut env = MapEnv::new()
        .with_var("value", "aaab")
        .with_var("upper", "Aaa");

    // `[[ == ]]` and case modification always recognize extglobs.
    check("$value == +(a)b", &mut env);
    check("${value^^@(a|b)} == AAAB", &mut env);

    // Removal and replacement consult the host's extglob option.
    check("${value##+(a)} == aaab", &mut env);
    env.options.insert("extglob".into(), true);
    check("${value##+(a)} == b", &mut env);

    // nocasematch affects conditional matching and replacement, but not
    // removal or case modification.
    env.options.insert("nocasematch".into(), true);
    check("$upper == a*", &mut env);
    check("${upper/a/X} == Xaa", &mut env);
    check("${upper#a*} == Aaa", &mut env);
    check("${upper,,a} == Aaa", &mut env);
}

#[test]
fn arithmetic_operands_support_full_scalar_expressions_and_mutation() {
    let mut env = MapEnv::new()
        .with_var("x", "2")
        .with_var("recursive", "x+1");
    check("'2**3**2' -eq 512", &mut env);
    check("'010 + 0x10 + 2#10 + 64#_' -eq 89", &mut env);
    check("recursive -eq 3", &mut env);
    check("'x=4,x++' -eq 4", &mut env);
    assert_eq!(env.var("x"), Some("5"));
    check("'x+=2' -eq 7", &mut env);
    assert_eq!(env.var("x"), Some("7"));
}

#[test]
fn unsupported_impure_parameter_forms_are_parse_errors() {
    assert!(matches!(
        parse("${value:=fallback}"),
        Err(ParseError::UnsupportedParameterExpansion { .. })
    ));
    assert!(matches!(
        parse("${value:?message}"),
        Err(ParseError::UnsupportedParameterExpansion { .. })
    ));
    assert!(matches!(
        parse("${value:}"),
        Err(ParseError::InvalidParameterExpansion { .. })
    ));
}

#[test]
fn bash_parameter_names_are_enabled_by_default() {
    let mut env = MapEnv::new()
        .with_var("#", "2")
        .with_var("?", "40")
        .with_var("-", "flags")
        .with_var("$", "1234")
        .with_var("!", "4321")
        .with_var("@", "args")
        .with_var("*", "args")
        .with_var("1", "first")
        .with_var("10", "tenth");

    check("${#} == 2", &mut env);
    check("${##} -eq 1", &mut env);
    check("${#?} -eq 2", &mut env);
    check("${?%0} == 4", &mut env);
    check("${#:-fallback} == 2", &mut env);
    check("${-} == flags", &mut env);
    check("${$} == 1234", &mut env);
    check("${!} == 4321", &mut env);
    check("${@} == args", &mut env);
    check("${*} == args", &mut env);
    check("${1} == first", &mut env);
    check("${10:-fallback} == tenth", &mut env);
}

#[test]
fn punctuation_only_names_require_opt_in() {
    for input in ["${}", "${.}", "${/.}", "${/}", "${//}", "${%}", "${^}"] {
        assert!(matches!(
            parse(input),
            Err(ParseError::InvalidParameterExpansion { .. })
        ));
    }

    let mut env = MapEnv::new()
        .with_var("", "path")
        .with_var("/", "basename")
        .with_var("//", "parent")
        .with_var(".", "path-no-ext")
        .with_var("/.", "basename-no-ext")
        .with_var("%", "percent")
        .with_var("^", "caret");

    check_with_punctuation_vars("${} == path", &mut env);
    check_with_punctuation_vars("${/} == basename", &mut env);
    check_with_punctuation_vars("${//} == parent", &mut env);
    check_with_punctuation_vars("${.} == path-no-ext", &mut env);
    check_with_punctuation_vars("${/.} == basename-no-ext", &mut env);
    check_with_punctuation_vars("${%} == percent", &mut env);
    check_with_punctuation_vars("${^} == caret", &mut env);
    check_with_punctuation_vars("\"${.}\" == path-no-ext", &mut env);
    check_with_punctuation_vars("${missing:-${.}} == path-no-ext", &mut env);
}

#[test]
fn malformed_names_remain_invalid_with_punctuation_opt_in() {
    let options = ParseOptions::default().punctuation_variables(true);
    for input in ["${12x}", "${foo.bar}", "${white space}", "${array[0]}"] {
        assert!(matches!(
            parse_with_options(input, options),
            Err(ParseError::InvalidParameterExpansion { .. })
        ));
    }

    assert!(matches!(
        parse("${missing:-${.}}"),
        Err(ParseError::InvalidParameterExpansion { .. })
    ));
}

#[test]
fn arithmetic_mutation_reports_a_read_only_host() {
    struct ReadOnlyEnv;

    impl Env for ReadOnlyEnv {
        fn var(&self, _name: &str) -> Option<&str> {
            None
        }
    }

    let expression = parse("'x=1' -eq 1").unwrap();
    let error = Evaluator::new(&mut ReadOnlyEnv, &StdFs)
        .eval(&expression)
        .unwrap_err();
    assert!(matches!(
        error,
        EvalError::ArithmeticAssignmentUnsupported(name) if name == "x"
    ));
}
