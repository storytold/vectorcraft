//! Module code (ECMA-262 2015 section 15.2): the import and export declarations at the top of a
//! module, read into a [`ModuleInfo`], and their early errors (section 15.2.1.1).
//!
//! A module is strict mode code throughout. An import binds its names as lexical declarations at
//! the top of the module, so a `var`, `let`, `const`, function or another import of the same name
//! is a redeclaration. An exported declaration is kept as the declaration; `export default` of an
//! expression or an anonymous function binds [`DEFAULT_BINDING`]. Each exported name may be
//! exported once, and a name an `export { … }` list exports from the module itself must be one it
//! declares or imports.

use std::collections::{HashMap, HashSet};

use super::{Parser, Result, describe};
use crate::ast::{
    Binding, DEFAULT_BINDING, ExportEntry, FunctionKind, ImportEntry, ImportName, LexKind, ModuleInfo, ModuleRequest, Name, Program, Stmt, StmtId,
    VarDecl,
};
use crate::token::{Keyword, Punct, Span, Token, TokenKind};

/// A name an `export { … }` list without `from` exports: the module's own binding (or import)
/// `local`, exported as `export_name`.
struct ClauseExport {
    export_name: String,
    local: String,
    span: Span,
}

#[derive(Eq)]
struct RequestUnits(Vec<u16>);

impl std::hash::Hash for RequestUnits {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(&self.0, state);
    }
}

impl PartialEq for RequestUnits {
    fn eq(&self, other: &Self) -> bool {
        #[cfg(test)]
        REQUEST_SEARCH_STEPS.with(|steps| steps.set(steps.get() + 1));
        self.0 == other.0
    }
}

#[derive(Eq)]
struct ImportLocal<'a>(&'a str);

impl std::hash::Hash for ImportLocal<'_> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(self.0, state);
    }
}

impl PartialEq for ImportLocal<'_> {
    fn eq(&self, other: &Self) -> bool {
        #[cfg(test)]
        EXPORT_SEARCH_STEPS.with(|steps| steps.set(steps.get() + 1));
        self.0 == other.0
    }
}

#[cfg(test)]
thread_local! {
    static REQUEST_SEARCH_STEPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static EXPORT_SEARCH_STEPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl Parser<'_> {
    /// Reads the source as a module: its statements, without its import and export declarations
    /// but with the declarations they export, and what it imports and exports.
    pub(crate) fn parse_module(mut self) -> Result<(Program, ModuleInfo, Vec<Token>)> {
        self.module = true;
        #[cfg(test)]
        REQUEST_SEARCH_STEPS.with(|steps| steps.set(0));
        let mut info = ModuleInfo::default();
        let mut requests = HashMap::new();
        let mut clause = Vec::new();
        let mut body = Vec::new();
        while self.tok.kind != TokenKind::Eof {
            if self.at_keyword(Keyword::Import) {
                self.import_declaration(&mut info, &mut requests)?;
            } else if self.at_keyword(Keyword::Export) {
                if let Some(stmt) = self.export_declaration(&mut info, &mut clause, &mut requests)? {
                    body.push(stmt);
                }
            } else {
                body.push(self.parse_statement_item()?);
            }
        }
        self.finish_exports(&mut info, clause)?;
        if let Some(tokens) = &mut self.tokens {
            tokens.push(self.tok.clone());
        }
        let program = Program { ast: self.ast, body, strict: true, directives: self.directives, lines: self.lines, es2015: self.es2015 };
        Ok((program, info, self.tokens.unwrap_or_default()))
    }

    /// The current token is the contextual keyword `word` (`as` or `from`), written without escapes.
    fn at_contextual(&self, word: &str) -> bool {
        self.tok.ident() == Some(word) && !self.tok.escaped
    }

    fn expect_contextual(&mut self, word: &str) -> Result<()> {
        if self.at_contextual(word) {
            self.bump()?;
            return Ok(());
        }
        Err(self.error_at_token(&self.tok, format!("expected '{word}' but found {}", describe(&self.tok))))
    }

    /// A module specifier: a string literal, added to the module's requests once. → its index.
    /// Any string value is a specifier here; the host decides which it can load.
    fn module_specifier(&mut self, info: &mut ModuleInfo, requests: &mut HashMap<RequestUnits, usize>) -> Result<usize> {
        let TokenKind::String(units) = &self.tok.kind else {
            return Err(self.error_at_token(&self.tok, format!("expected a module specifier string but found {}", describe(&self.tok))));
        };
        if self.tok.legacy_octal {
            return Err(self.error_at_token(&self.tok, "octal escape sequences are not allowed in strict mode"));
        }
        let units = RequestUnits(units.to_vec());
        let span = self.bump()?.span;
        if let Some(index) = requests.get(&units) {
            return Ok(*index);
        }
        let specifier = String::from_utf16_lossy(&units.0);
        let index = info.requests.len();
        info.requests.push(ModuleRequest { specifier, units: units.0.clone(), span });
        requests.insert(units, index);
        Ok(info.requests.len() - 1)
    }

    /// A name as an import or export list gives it: any identifier name, keywords included. → the
    /// name and whether it was an identifier (not a keyword).
    fn list_name(&mut self) -> Result<(Name, bool)> {
        let Some(name) = self.tok.identifier_name().map(str::to_string) else {
            return Err(self.error_at_token(&self.tok, format!("expected a name but found {}", describe(&self.tok))));
        };
        let ident = matches!(self.tok.kind, TokenKind::Ident(_));
        let token = self.bump()?;
        Ok((Name { name, span: token.span }, ident))
    }

    /// `import …;` (section 15.2.2). Each name it binds is declared in the module's scope.
    fn import_declaration(&mut self, info: &mut ModuleInfo, requests: &mut HashMap<RequestUnits, usize>) -> Result<()> {
        let import = self.bump()?;
        if self.at(Punct::LParen) {
            return Err(self.error_at_token(&import, "import() is not supported"));
        }
        if matches!(self.tok.kind, TokenKind::String(_)) {
            self.module_specifier(info, requests)?;
            return self.semicolon();
        }
        let mut bound: Vec<(ImportName, Name)> = Vec::new();
        let mut list = true;
        if matches!(self.tok.kind, TokenKind::Ident(_)) {
            let local = self.binding_identifier()?;
            bound.push((ImportName::Name("default".into()), local));
            list = self.eat(Punct::Comma)?;
        }
        if list {
            if self.eat(Punct::Star)? {
                self.expect_contextual("as")?;
                let local = self.binding_identifier()?;
                bound.push((ImportName::Namespace, local));
            } else {
                self.expect(Punct::LBrace)?;
                while !self.at(Punct::RBrace) {
                    if matches!(self.tok.kind, TokenKind::Ident(_)) && !self.next_is_as() {
                        // A name without `as` binds itself, so it must be one a declaration may bind.
                        let local = self.binding_identifier()?;
                        bound.push((ImportName::Name(local.name.clone()), local));
                    } else {
                        let (imported, _) = self.list_name()?;
                        self.expect_contextual("as")?;
                        let local = self.binding_identifier()?;
                        bound.push((ImportName::Name(imported.name), local));
                    }
                    if !self.eat(Punct::Comma)? {
                        break;
                    }
                }
                self.expect(Punct::RBrace)?;
            }
        }
        self.expect_contextual("from")?;
        let request = self.module_specifier(info, requests)?;
        self.semicolon()?;
        for (import, local) in bound {
            self.declare_lexical(&local, false)?;
            info.imports.push(ImportEntry { request, import, local });
        }
        Ok(())
    }

    /// The token after the current one is the contextual keyword `as`.
    fn next_is_as(&self) -> bool {
        self.peek().is_some_and(|t| !t.escaped && t.ident() == Some("as"))
    }

    /// `export …` (section 15.2.3). → the declaration it exports, which runs in place.
    fn export_declaration(
        &mut self,
        info: &mut ModuleInfo,
        clause: &mut Vec<ClauseExport>,
        requests: &mut HashMap<RequestUnits, usize>,
    ) -> Result<Option<StmtId>> {
        let export = self.bump()?;
        let span = export.span;
        if self.eat(Punct::Star)? {
            if self.at_contextual("as") {
                self.bump()?;
                let (name, _) = self.list_name()?;
                self.expect_contextual("from")?;
                let request = self.module_specifier(info, requests)?;
                self.semicolon()?;
                info.exports.push(ExportEntry::Indirect { export_name: name.name, request, import: ImportName::Namespace, span: name.span });
                return Ok(None);
            }
            self.expect_contextual("from")?;
            let request = self.module_specifier(info, requests)?;
            self.semicolon()?;
            info.exports.push(ExportEntry::Star { request, span });
            return Ok(None);
        }
        if self.at(Punct::LBrace) {
            self.bump()?;
            // Each name: the name it refers to (and whether that was an identifier) and the name it
            // is exported as.
            let mut names: Vec<(Name, bool, Name)> = Vec::new();
            while !self.at(Punct::RBrace) {
                let (local, ident) = self.list_name()?;
                let exported = if self.at_contextual("as") {
                    self.bump()?;
                    self.list_name()?.0
                } else {
                    local.clone()
                };
                names.push((local, ident, exported));
                if !self.eat(Punct::Comma)? {
                    break;
                }
            }
            self.expect(Punct::RBrace)?;
            if self.at_contextual("from") {
                self.bump()?;
                let request = self.module_specifier(info, requests)?;
                self.semicolon()?;
                for (local, _, exported) in names {
                    let import = ImportName::Name(local.name);
                    info.exports.push(ExportEntry::Indirect { export_name: exported.name, request, import, span: exported.span });
                }
                return Ok(None);
            }
            self.semicolon()?;
            for (local, ident, exported) in names {
                // Without `from`, each name refers to a binding of the module (section 15.2.3.1).
                if !ident || crate::token::strict_reserved(&local.name) {
                    return Err(self.error_at_offset(local.span.start, format!("'{}' cannot be exported as a binding", local.name)));
                }
                clause.push(ClauseExport { export_name: exported.name, local: local.name, span: local.span });
            }
            return Ok(None);
        }
        if self.at_keyword(Keyword::Default) {
            self.bump()?;
            return self.export_default(info, span).map(Some);
        }
        let exportable = self.at_keyword(Keyword::Var) || self.at_keyword(Keyword::Function) || self.at_lexical_declaration().is_some();
        if !exportable {
            return Err(self.error_at_token(&self.tok, format!("expected a declaration after 'export' but found {}", describe(&self.tok))));
        }
        let stmt = self.parse_statement_item()?;
        let names: Vec<Name> = match self.ast.stmt(stmt) {
            Some(Stmt::Var(decls) | Stmt::Lexical(_, decls)) => decls.iter().flat_map(|d| self.ast.bound_names(&d.target)).collect(),
            Some(Stmt::FunctionDecl(fid)) => self.ast.function(*fid).and_then(|f| f.name.clone()).into_iter().collect(),
            _ => return Err(self.error_at_offset(span.start, "expected a declaration after 'export'")),
        };
        for name in names {
            info.exports.push(ExportEntry::Local { export_name: name.name.clone(), local: name.name, span: name.span });
        }
        Ok(Some(stmt))
    }

    /// `export default …` after `default`: a function declaration (named or not) or an expression.
    fn export_default(&mut self, info: &mut ModuleInfo, span: Span) -> Result<StmtId> {
        if self.at_keyword(Keyword::Class) {
            return Err(self.error_at_token(&self.tok, "classes are not supported"));
        }
        if self.at_keyword(Keyword::Function) {
            self.enter()?;
            let result = self.default_function();
            self.leave();
            let (stmt, fid, local) = result?;
            self.declare_function(fid)?;
            info.exports.push(ExportEntry::Local { export_name: "default".into(), local, span });
            return Ok(stmt);
        }
        let start = self.tok.span;
        let value = self.parse_assignment(false)?;
        self.semicolon()?;
        let target = Binding::Name(Name { name: DEFAULT_BINDING.into(), span: start });
        let stmt = self.ast.push_stmt(Stmt::Lexical(LexKind::Const, vec![VarDecl { target, init: Some(value) }]), self.span_from(start));
        info.exports.push(ExportEntry::Local { export_name: "default".into(), local: DEFAULT_BINDING.into(), span });
        Ok(stmt)
    }

    /// `function name(…) {…}` or `function (…) {…}` after `export default`: a declaration of
    /// `name`, or of [`DEFAULT_BINDING`] for a function without one.
    fn default_function(&mut self) -> Result<(StmtId, crate::ast::FunctionId, String)> {
        let keyword = self.expect_keyword(Keyword::Function)?;
        let start = keyword.span;
        let name = if matches!(self.tok.kind, TokenKind::Ident(_)) {
            self.function_name()?
        } else {
            (Name { name: DEFAULT_BINDING.into(), span: start }, keyword)
        };
        let local = name.0.name.clone();
        let fid = self.parse_function_rest(FunctionKind::Declaration, Some(name), start)?;
        let stmt = self.ast.push_stmt(Stmt::FunctionDecl(fid), self.span_from(start));
        Ok((stmt, fid, local))
    }

    /// The early errors of the exports as a whole (section 15.2.1.1): no name exported twice, and
    /// each name an `export { … }` list takes from the module itself declared or imported there. An
    /// exported named import becomes a re-export of its imported binding (section 15.2.1.16.1).
    /// A namespace import keeps its local binding.
    fn finish_exports(&self, info: &mut ModuleInfo, clause: Vec<ClauseExport>) -> Result<()> {
        #[cfg(test)]
        EXPORT_SEARCH_STEPS.with(|steps| steps.set(0));
        let mut imports = HashMap::with_capacity(info.imports.len());
        for import in &info.imports {
            imports.entry(ImportLocal(&import.local.name)).or_insert(import);
        }
        let top = self.scopes.first();
        for c in clause {
            let declared = top.is_some_and(|s| s.lexical.contains_key(&c.local) || s.vars.contains(&c.local));
            if !declared {
                return Err(self.error_at_offset(c.span.start, format!("'{}' is exported but not declared in the module", c.local)));
            }
            let imported = imports.get(&ImportLocal(&c.local));
            let entry = match imported {
                Some(i) if matches!(&i.import, ImportName::Name(_)) => {
                    ExportEntry::Indirect { export_name: c.export_name, request: i.request, import: i.import.clone(), span: c.span }
                }
                _ => ExportEntry::Local { export_name: c.export_name, local: c.local, span: c.span },
            };
            info.exports.push(entry);
        }
        let mut seen = HashSet::new();
        for e in &info.exports {
            let (name, span) = match e {
                ExportEntry::Local { export_name, span, .. } | ExportEntry::Indirect { export_name, span, .. } => (export_name, span),
                ExportEntry::Star { .. } => continue,
            };
            if !seen.insert(name.as_str()) {
                return Err(self.error_at_offset(span.start, format!("'{name}' is exported more than once")));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{EXPORT_SEARCH_STEPS, REQUEST_SEARCH_STEPS};
    use crate::{Options, parse_module};

    fn run_namespace_modules(main: &str, files: &[(&str, &str)]) -> (Result<crate::JsValue, crate::RunError>, crate::Realm) {
        let mut realm = crate::Realm::with_options(Options { es2015: true, ..Options::default() }, crate::Limits::default());
        realm.run("var out = [];").unwrap();
        let mut load = |_: &str, specifier: &str| {
            files.iter().find(|(name, _)| *name == specifier).map_or_else(
                || Err(crate::LoadError::Other(format!("missing module {specifier}"))),
                |(_, source)| Ok(crate::LoadedModule { key: specifier.into(), source: Some((*source).into()) }),
            )
        };
        let result = realm.run_module("main.mjs", main, crate::ModuleLimits::default(), &mut load);
        (result, realm)
    }

    #[test]
    fn namespace_reexport_bindings_keep_their_origin() {
        let files = [
            ("./a.mjs", "import * as ns from './shared.mjs'; export {ns};"),
            ("./b.mjs", "import * as ns from './shared.mjs'; export {ns};"),
            ("./barrel.mjs", "export * from './a.mjs'; export * from './b.mjs';"),
            ("./shared.mjs", "export var value = 7;"),
        ];
        let (result, _) = run_namespace_modules("import {ns} from './barrel.mjs'; out.push(ns.value);", &files);
        assert!(result.as_ref().is_err_and(|error| error.message.contains("ambiguous")), "{result:?}");
        let files = [
            ("./a.mjs", "import {value} from './shared.mjs'; export {value};"),
            ("./b.mjs", "import {value} from './shared.mjs'; export {value};"),
            ("./barrel.mjs", "export * from './a.mjs'; export * from './b.mjs';"),
            ("./shared.mjs", "export var value = 7; export function change() { value = 9; }"),
        ];
        let (result, mut realm) =
            run_namespace_modules("import {value} from './barrel.mjs'; import {change} from './shared.mjs'; change(); out.push(value);", &files);
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(realm.run("out.join('|')"), Ok(crate::JsValue::String("9".into())));
    }

    #[test]
    fn namespace_reexports_link_through_cycles() {
        let files = [("./a.mjs", "import * as ns from './b.mjs'; export {ns};"), ("./b.mjs", "import * as ns from './a.mjs'; export {ns};")];
        let (result, mut realm) = run_namespace_modules("import * as a from './a.mjs'; out.push(a.ns.ns === a);", &files);
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(realm.run("out.join('|')"), Ok(crate::JsValue::String("true".into())));
    }

    #[test]
    fn module_specifiers_reject_legacy_octal_escapes() {
        for source in
            [r"import './\141.mjs';", r"import value from './\141.mjs';", r"export * from './\141.mjs';", r"export { value } from './\141.mjs';"]
        {
            assert!(parse_module(source, &Options::default()).is_err(), "{source}");
        }
        for source in [r"import './\u0061.mjs';", r"export * from './\x61.mjs';", r"import './\uD800.mjs';"] {
            assert!(parse_module(source, &Options::default()).is_ok(), "{source}");
        }
    }

    #[test]
    fn module_request_search_work_is_bounded_by_request_count() {
        let count = 1_000;
        let mut source = String::new();
        for index in 0..count {
            source.push_str(&format!("import './{index}.mjs';"));
        }
        source.push_str(r"import './\u0030.mjs'; import './\uD800.mjs'; import './\uFFFD.mjs'; import './\uDC00.mjs'; import './\uD800.mjs';");
        let (_, info) = match parse_module(&source, &Options::default()) {
            Ok(parsed) => parsed,
            Err(error) => panic!("{error:?}"),
        };
        assert_eq!(info.requests.len(), count + 3);
        for (index, request) in info.requests.iter().take(count).enumerate() {
            assert_eq!(request.specifier, format!("./{index}.mjs"));
        }
        assert_eq!(info.requests[count].units[2], 0xD800);
        assert_eq!(info.requests[count + 1].units[2], 0xFFFD);
        assert_eq!(info.requests[count + 2].units[2], 0xDC00);
        let steps = REQUEST_SEARCH_STEPS.with(std::cell::Cell::get);
        assert!(steps > 0, "the repeated requests compare their exact UTF-16 keys");
        assert!(steps <= 2 * (count + 5), "{steps} request-search steps for {} requests", count + 5);
    }

    #[test]
    fn imported_export_resolution_work_is_bounded_by_binding_count() {
        let count = 1_000;
        let bindings = (0..count).map(|index| format!("value as local{index}")).collect::<Vec<_>>().join(",");
        let exports = (0..count).map(|index| format!("local{index} as exported{index}")).collect::<Vec<_>>().join(",");
        let source = format!(
            "import {{{bindings}}} from './values.mjs'; import * as ns from './namespace.mjs'; export {{{exports},ns as namespace,local0 as again}};"
        );
        let (_, info) = match parse_module(&source, &Options::default()) {
            Ok(parsed) => parsed,
            Err(error) => panic!("{error:?}"),
        };
        assert_eq!(info.imports.len(), count + 1);
        assert_eq!(info.exports.len(), count + 2);
        for (index, export) in info.exports.iter().take(count).enumerate() {
            assert!(matches!(export, crate::ExportEntry::Indirect { export_name, request: 0, import: crate::ast::ImportName::Name(name), .. }
                    if export_name == &format!("exported{index}") && name == "value"));
        }
        assert!(matches!(&info.exports[count], crate::ExportEntry::Local { export_name, local, .. } if export_name == "namespace" && local == "ns"));
        assert!(
            matches!(&info.exports[count + 1], crate::ExportEntry::Indirect { request: 0, import: crate::ast::ImportName::Name(name), .. } if name == "value")
        );
        let steps = EXPORT_SEARCH_STEPS.with(std::cell::Cell::get);
        assert!(steps > 0);
        assert!(steps <= 3 * (count + 2), "{steps} import-local comparisons for {} exports", count + 2);
        for source in [
            "import {value as x} from './values.mjs'; export {x,x};",
            "import {value as x} from './values.mjs'; import * as x from './namespace.mjs';",
        ] {
            assert!(parse_module(source, &Options::default()).is_err(), "{source}");
        }
    }
}
