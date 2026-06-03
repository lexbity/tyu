use crate::util::{slice_span, try_load_module_file};
use frontend::{
    lex::Lexer,
    parse::{DeclAst, DeclKind, ModuleAst, Parser},
    span::Span,
    token::TokenKind,
};

pub fn check_program(module: &ModuleAst, src: &[u8], search_dirs: &[&[u8]]) -> Result<(), u32> {
    let root_name = slice_span(src, module.name);
    if let Some(def_src) = try_load_module_file(search_dirs, root_name, b".def") {
        let def_ast = Parser::new(def_src.as_slice())
            .parse_module_ast()
            .map_err(|_| 2300u32)?;
        check_iface(&def_ast, def_src.as_slice(), module, src)?;
    }

    for imp in module.imports.iter() {
        let mname = slice_span(src, imp.module);
        let def_src = try_load_module_file(search_dirs, mname, b".def").ok_or(2201u32)?;
        let def_ast = Parser::new(def_src.as_slice())
            .parse_module_ast()
            .map_err(|_| 2202u32)?;

        for sym in imp.names.iter() {
            let sym_name = slice_span(src, *sym);
            if !is_exported(&def_ast, def_src.as_slice(), sym_name) {
                return Err(2203u32);
            }
        }

        if let Some(mod_src) = try_load_module_file(search_dirs, mname, b".mod") {
            let mod_ast = Parser::new(mod_src.as_slice())
                .parse_module_ast()
                .map_err(|_| 2204u32)?;
            check_iface(&def_ast, def_src.as_slice(), &mod_ast, mod_src.as_slice())?;
        }
    }

    Ok(())
}

pub fn check_iface(
    def_ast: &ModuleAst,
    def_src: &[u8],
    mod_ast: &ModuleAst,
    mod_src: &[u8],
) -> Result<(), u32> {
    for name in export_iter(def_ast, def_src) {
        if !is_exported(mod_ast, mod_src, name) {
            return Err(2210u32);
        }
    }
    for name in export_iter(mod_ast, mod_src) {
        if !is_exported(def_ast, def_src, name) {
            return Err(2211u32);
        }
    }

    for name in export_iter(def_ast, def_src) {
        let def_decl = find_decl(def_ast, def_src, name).ok_or(2212u32)?;
        let mod_decl = find_decl(mod_ast, mod_src, name).ok_or(2213u32)?;

        if def_decl.kind != mod_decl.kind {
            return Err(2214u32);
        }
        if !attrs_eq(def_src, &def_decl.attrs, mod_src, &mod_decl.attrs) {
            return Err(2215u32);
        }
        if def_decl.kind == DeclKind::Word {
            let def_sig = def_decl.sig.ok_or(2216u32)?;
            let mod_sig = mod_decl.sig.ok_or(2217u32)?;
            if !sig_eq(slice_span(def_src, def_sig), slice_span(mod_src, mod_sig)) {
                return Err(2218u32);
            }
            if def_decl.effect_bits != mod_decl.effect_bits {
                return Err(2219u32);
            }
        }
    }

    Ok(())
}

pub fn sig_eq(a: &[u8], b: &[u8]) -> bool {
    let mut la = Lexer::new(a);
    let mut lb = Lexer::new(b);
    loop {
        let ta = la.next();
        let tb = lb.next();
        if ta.kind != tb.kind {
            return false;
        }
        if ta.kind == TokenKind::Eof {
            return true;
        }
        match ta.kind {
            TokenKind::Ident | TokenKind::Number | TokenKind::String | TokenKind::EffectSet => {
                let sa = &a[ta.span.start..ta.span.end];
                let sb = &b[tb.span.start..tb.span.end];
                if sa != sb {
                    return false;
                }
            }
            _ => {}
        }
    }
}

pub fn attrs_eq(
    def_src: &[u8],
    def_attrs: &frontend::fixed::FixedVec<Span, 16>,
    mod_src: &[u8],
    mod_attrs: &frontend::fixed::FixedVec<Span, 16>,
) -> bool {
    if def_attrs.len() != mod_attrs.len() {
        return false;
    }
    for i in 0..def_attrs.len() {
        let da = *def_attrs.get(i).expect("len checked above");
        let ma = *mod_attrs.get(i).expect("len checked above");
        if slice_span(def_src, da) != slice_span(mod_src, ma) {
            return false;
        }
    }
    true
}

pub fn export_iter<'a>(ast: &'a ModuleAst, src: &'a [u8]) -> ExportIter<'a> {
    ExportIter { ast, src, i: 0 }
}

pub struct ExportIter<'a> {
    ast: &'a ModuleAst,
    src: &'a [u8],
    i: usize,
}

impl<'a> Iterator for ExportIter<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        if self.ast.has_export_stmt {
            let span = *self.ast.exports.get(self.i)?;
            self.i += 1;
            Some(slice_span(self.src, span))
        } else {
            let decl = self.ast.decls.get(self.i)?;
            self.i += 1;
            Some(slice_span(self.src, decl.name))
        }
    }
}

pub fn is_exported(ast: &ModuleAst, src: &[u8], name: &[u8]) -> bool {
    if ast.has_export_stmt {
        for s in ast.exports.iter() {
            if slice_span(src, *s) == name {
                return true;
            }
        }
        false
    } else {
        find_decl(ast, src, name).is_some()
    }
}

pub fn find_decl<'a>(ast: &'a ModuleAst, src: &'a [u8], name: &[u8]) -> Option<&'a DeclAst> {
    ast.decls.iter().find(|d| slice_span(src, d.name) == name)
}

pub fn find_word_decl<'a>(m: &'a ModuleAst, src: &[u8], name: &[u8]) -> Option<&'a DeclAst> {
    for d in m.decls.iter() {
        if d.kind != DeclKind::Word {
            continue;
        }
        if slice_span(src, d.name) == name {
            return Some(d);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use frontend::fixed::FixedVec;
    use frontend::parse::Parser;
    use frontend::span::Span;

    fn parse(src: &str) -> ModuleAst {
        Parser::new(src.as_bytes()).parse_module_ast().unwrap()
    }

    fn make_attrs(spans: &[Span]) -> FixedVec<Span, 16> {
        let mut v = FixedVec::new();
        for &s in spans {
            v.push(s).unwrap();
        }
        v
    }

    // -- sig_eq --

    #[test]
    fn test_sig_eq_identical() {
        assert!(sig_eq(b"( i64 -- i64 )", b"( i64 -- i64 )"));
    }

    #[test]
    fn test_sig_eq_different_inputs() {
        assert!(!sig_eq(b"( i64 -- bool )", b"( bool -- bool )"));
    }

    #[test]
    fn test_sig_eq_empty() {
        assert!(sig_eq(b"( -- )", b"( -- )"));
    }

    // -- attrs_eq --

    #[test]
    fn test_attrs_eq_identical() {
        let def = make_attrs(&[Span::new(0, 3), Span::new(5, 8)]);
        let mo = make_attrs(&[Span::new(0, 3), Span::new(5, 8)]);
        assert!(attrs_eq(b"foobar", &def, b"foobar", &mo));
    }

    #[test]
    fn test_attrs_eq_different() {
        let def = make_attrs(&[Span::new(0, 3)]);
        let mo = make_attrs(&[Span::new(0, 4)]);
        assert!(!attrs_eq(b"foobar", &def, b"foobar", &mo));
    }

    // -- is_exported --

    #[test]
    fn test_is_exported_with_export_stmt() {
        let src = b"module m; export { foo } ; : bar ; : foo ; end;";
        let ast = parse(core::str::from_utf8(src).unwrap());
        assert!(is_exported(&ast, src, b"foo"));
        assert!(!is_exported(&ast, src, b"bar"));
    }

    #[test]
    fn test_is_exported_decls_only() {
        let src = b"module m; : foo ; : bar ; end;";
        let ast = parse(core::str::from_utf8(src).unwrap());
        assert!(is_exported(&ast, src, b"foo"));
        assert!(is_exported(&ast, src, b"bar"));
    }

    // -- find_decl --

    #[test]
    fn test_find_decl_found() {
        let src = b"module m; : foo ; end;";
        let ast = parse(core::str::from_utf8(src).unwrap());
        assert!(find_decl(&ast, src, b"foo").is_some());
    }

    #[test]
    fn test_find_decl_not_found() {
        let src = b"module m; : foo ; end;";
        let ast = parse(core::str::from_utf8(src).unwrap());
        assert!(find_decl(&ast, src, b"bar").is_none());
    }

    // -- export_iter --

    #[test]
    fn test_export_iter_with_export_stmt() {
        let src = b"module m; export { foo, bar } ; : baz ; end;";
        let ast = parse(core::str::from_utf8(src).unwrap());
        let names: Vec<&[u8]> = export_iter(&ast, src).collect();
        assert_eq!(names.len(), 2);
        assert!(names.contains(&b"foo"));
    }

    #[test]
    fn test_export_iter_no_export_stmt() {
        let src = b"module m; : foo ; : bar ; end;";
        let ast = parse(core::str::from_utf8(src).unwrap());
        let names: Vec<&[u8]> = export_iter(&ast, src).collect();
        assert_eq!(names.len(), 2);
    }

    // -- iface_error_message --

    #[test]
    fn test_error_messages_all_codes() {
        let codes: &[u32] = &[
            2020, 2021, 2022, 2201, 2202, 2203, 2204, 2205, 2207, 2210, 2211, 2212, 2213, 2214,
            2215, 2216, 2217, 2218, 2219, 2220, 2223, 2300,
        ];
        for &code in codes {
            assert!(
                !iface_error_message(code).is_empty(),
                "code {code} has empty message"
            );
        }
    }

    #[test]
    fn test_error_message_unknown() {
        assert_eq!(iface_error_message(9999), b"interface/import error");
    }
}

pub fn iface_error_message(code: u32) -> &'static [u8] {
    match code {
        // Module and subtype errors (from driver.rs)
        2020 => b"too many subtypes (max 64)",
        2021 => b"subtype name too long",
        2022 => b"base type name too long",
        // `check_program` import/interface errors.
        2201 => b"import interface file (.def) not found",
        2202 => b"failed to parse imported interface (.def)",
        2203 => b"imported symbol not exported by interface",
        2204 => b"failed to parse imported implementation (.mod)",
        2205 => b"failed to parse signature in imported interface",
        2207 => b"too many imported words (max 256 total)",
        2210 => b"implementation missing exported symbol from interface",
        2211 => b"implementation exports symbol not present in interface",
        2212 => b"interface export refers to missing declaration",
        2213 => b"implementation export refers to missing declaration",
        2214 => b"interface/implementation declaration kind mismatch",
        2215 => b"interface/implementation attributes mismatch",
        2216 => b"interface exported word missing signature",
        2217 => b"implementation exported word missing signature",
        2218 => b"interface/implementation word signature mismatch",
        2219 => b"interface/implementation word effect mismatch",
        2220 => b"local word name too long",
        2223 => b"too many words in module (max 256)",
        2300 => b"failed to parse module interface (.def) for current module",
        _ => b"interface/import error",
    }
}
