use frontend::{
    lex::Lexer,
    parse::{DeclAst, DeclKind, ModuleAst, Parser},
    span::Span,
    token::TokenKind,
};
use crate::util::{slice_span, try_load_module_file};

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
            if def_decl.effect_suspend != mod_decl.effect_suspend {
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
        let da = *def_attrs.get(i).unwrap();
        let ma = *mod_attrs.get(i).unwrap();
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
    for d in ast.decls.iter() {
        if slice_span(src, d.name) == name {
            return Some(d);
        }
    }
    None
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

pub fn iface_error_message(code: u32) -> &'static [u8] {
    match code {
        // `check_program` import/interface errors (stable codes).
        2201 => b"import interface file (.def) not found",
        2202 => b"failed to parse imported interface (.def)",
        2203 => b"imported symbol not exported by interface",
        2204 => b"failed to parse imported implementation (.mod)",
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
        2300 => b"failed to parse module interface (.def) for current module",
        _ => b"interface/import error",
    }
}
