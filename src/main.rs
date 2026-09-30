use std::{
    collections::{BTreeSet, HashSet},
    fs,
    path::{Path, PathBuf},
    process,
};

use clap::{Parser, ValueEnum};
use proc_macro2::{Ident, LineColumn, Span};
use serde::Serialize;
use syn::{
    Attribute, Block, Expr, ExprIf, Meta, Token,
    punctuated::Punctuated,
    spanned::Spanned,
    visit::{self, Visit},
};

#[derive(Debug, Default, PartialEq, Eq)]
struct Counts {
    code: usize,
    comments: usize,
    tests: usize,
    complexity: usize,
}

#[derive(Debug, Parser)]
#[command(name = "rsloc", about = "Count Rust source lines")]
struct Cli {
    #[arg(
        long,
        value_name = "strings",
        value_delimiter = ',',
        default_values = [".git", ".hg", ".svn", "target"]
    )]
    exclude_dir: Vec<String>,

    #[arg(short = 'n', long, value_name = "strings", value_delimiter = ',')]
    exclude_file: Vec<String>,

    #[arg(short = 'f', long, value_enum, default_value = "tabular")]
    format: OutputFormat,

    /// List each file's items instead of totalling the file
    #[arg(short = 'i', long)]
    items: bool,

    #[arg(value_name = "FILES OR DIRECTORIES", default_value = ".")]
    paths: Vec<PathBuf>,
}

#[derive(Clone, Debug, Serialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
enum OutputFormat {
    Tabular,
    Json,
}

struct Options {
    exclude_dirs: HashSet<String>,
    exclude_files: HashSet<String>,
}

impl From<&Cli> for Options {
    fn from(cli: &Cli) -> Self {
        Self {
            exclude_dirs: cli
                .exclude_dir
                .iter()
                .map(|value| value.to_string())
                .collect(),
            exclude_files: cli
                .exclude_file
                .iter()
                .map(|value| value.to_string())
                .collect(),
        }
    }
}

#[derive(Debug, Serialize)]
struct FileCounts {
    file: String,
    prod: usize,
    test: usize,
    doc: usize,
    cog: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    items: Option<Vec<ItemCounts>>,
}

#[derive(Debug, PartialEq, Eq, Serialize)]
struct ItemCounts {
    name: String,
    kind: &'static str,
    line: usize,
    prod: usize,
    doc: usize,
    cog: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    items: Vec<ItemCounts>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let cli = Cli::parse();
    let options = Options::from(&cli);
    let current_dir = fs::canonicalize(".").map_err(|error| error.to_string())?;
    let mut paths = Vec::new();

    for input in &cli.paths {
        let path =
            fs::canonicalize(input).map_err(|error| format!("{}: {error}", input.display()))?;
        collect_files(&path, &options, &mut paths)?;
    }

    paths.sort();
    paths.dedup();

    let mut files = Vec::with_capacity(paths.len());
    for path in paths {
        let source =
            fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        let parsed = ParsedSource::parse(&source)
            .map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
        let counts = parsed.counts();
        let file = path
            .strip_prefix(&current_dir)
            .unwrap_or(&path)
            .display()
            .to_string();

        files.push(FileCounts {
            file,
            prod: counts.code,
            test: counts.tests,
            doc: counts.comments,
            cog: counts.complexity,
            items: cli.items.then(|| parsed.items()),
        });
    }

    match cli.format {
        OutputFormat::Tabular if cli.items => print_items_tabular(&files),
        OutputFormat::Tabular => print_tabular(&files),
        OutputFormat::Json => print_json(&files)?,
    }

    Ok(())
}

fn collect_files(path: &Path, options: &Options, files: &mut Vec<PathBuf>) -> Result<(), String> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| format!("{}: {error}", path.display()))?;

    if metadata.is_dir() {
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| options.exclude_dirs.contains(name))
        {
            return Ok(());
        }

        let entries = fs::read_dir(path).map_err(|error| format!("{}: {error}", path.display()))?;
        for entry in entries {
            let entry = entry.map_err(|error| format!("{}: {error}", path.display()))?;
            collect_files(&entry.path(), options, files)?;
        }
    } else if metadata.is_file() && should_include_file(path, options) {
        files.push(path.to_path_buf());
    }

    Ok(())
}

fn should_include_file(path: &Path, options: &Options) -> bool {
    let file_name = path.file_name().and_then(|name| name.to_str());
    if file_name.is_some_and(|name| options.exclude_files.contains(name)) {
        return false;
    }

    let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
        return false;
    };

    extension.eq_ignore_ascii_case("rs")
}

fn print_tabular(files: &[FileCounts]) {
    let prod_total: usize = files.iter().map(|file| file.prod).sum();
    let test_total: usize = files.iter().map(|file| file.test).sum();
    let doc_total: usize = files.iter().map(|file| file.doc).sum();

    let file_width = files
        .iter()
        .map(|file| file.file.len())
        .max()
        .unwrap_or(4)
        .max(4);
    let prod_width = files
        .iter()
        .map(|file| file.prod)
        .chain([prod_total])
        .map(|value| value.to_string().len())
        .max()
        .unwrap_or(4)
        .max(4);
    let test_width = files
        .iter()
        .map(|file| file.test)
        .chain([test_total])
        .map(|value| value.to_string().len())
        .max()
        .unwrap_or(4)
        .max(4);
    let doc_width = files
        .iter()
        .map(|file| file.doc)
        .chain([doc_total])
        .map(|value| value.to_string().len())
        .max()
        .unwrap_or(3)
        .max(3);
    let cog_width = files
        .iter()
        .map(|file| file.cog)
        .map(|value| value.to_string().len())
        .max()
        .unwrap_or(3)
        .max(3);

    let rule = "─".repeat(file_width + prod_width + test_width + doc_width + cog_width + 8);

    println!(
        "{:<file_width$}  {:>prod_width$}  {:>test_width$}  {:>doc_width$}  {:>cog_width$}",
        "File", "Prod", "Test", "Doc", "Cog"
    );
    println!("{rule}");
    for file in files {
        println!(
            "{:<file_width$}  {:>prod_width$}  {:>test_width$}  {:>doc_width$}  {:>cog_width$}",
            file.file, file.prod, file.test, file.doc, file.cog
        );
    }
    println!("{rule}");
    println!(
        "{:<file_width$}  {:>prod_width$}  {:>test_width$}  {:>doc_width$}",
        "", prod_total, test_total, doc_total
    );
}

struct ItemRow<'a> {
    location: String,
    name: String,
    item: &'a ItemCounts,
}

fn print_items_tabular(files: &[FileCounts]) {
    let mut rows = Vec::new();
    for file in files {
        if let Some(items) = &file.items {
            flatten_items(&file.file, items, 0, &mut rows);
        }
    }

    // The totals are the file totals, which also count lines outside any
    // listed item, such as `use` declarations.
    let prod_total: usize = files.iter().map(|file| file.prod).sum();
    let doc_total: usize = files.iter().map(|file| file.doc).sum();

    let location_width = column_width(
        "Location",
        rows.iter().map(|row| row.location.chars().count()),
    );
    let name_width = column_width("Item", rows.iter().map(|row| row.name.chars().count()));
    let kind_width = column_width("Kind", rows.iter().map(|row| row.item.kind.len()));
    let prod_width = column_width(
        "Prod",
        rows.iter()
            .map(|row| row.item.prod)
            .chain([prod_total])
            .map(digits),
    );
    let doc_width = column_width(
        "Doc",
        rows.iter()
            .map(|row| row.item.doc)
            .chain([doc_total])
            .map(digits),
    );
    let cog_width = column_width("Cog", rows.iter().map(|row| digits(row.item.cog)));

    let rule = "─"
        .repeat(location_width + name_width + kind_width + prod_width + doc_width + cog_width + 10);

    println!(
        "{:<location_width$}  {:<name_width$}  {:<kind_width$}  {:>prod_width$}  {:>doc_width$}  {:>cog_width$}",
        "Location", "Item", "Kind", "Prod", "Doc", "Cog"
    );
    println!("{rule}");
    for row in &rows {
        println!(
            "{:<location_width$}  {:<name_width$}  {:<kind_width$}  {:>prod_width$}  {:>doc_width$}  {:>cog_width$}",
            row.location, row.name, row.item.kind, row.item.prod, row.item.doc, row.item.cog
        );
    }
    println!("{rule}");
    println!(
        "{:<location_width$}  {:<name_width$}  {:<kind_width$}  {:>prod_width$}  {:>doc_width$}",
        "", "", "", prod_total, doc_total
    );
}

fn flatten_items<'a>(
    file: &str,
    items: &'a [ItemCounts],
    depth: usize,
    rows: &mut Vec<ItemRow<'a>>,
) {
    for item in items {
        rows.push(ItemRow {
            location: format!("{file}:{}", item.line),
            name: format!("{:indent$}{}", "", item.name, indent = depth * 2),
            item,
        });
        flatten_items(file, &item.items, depth + 1, rows);
    }
}

fn column_width(header: &str, widths: impl Iterator<Item = usize>) -> usize {
    widths.max().unwrap_or(0).max(header.len())
}

fn digits(value: usize) -> usize {
    value.to_string().len()
}

fn print_json(files: &[FileCounts]) -> Result<(), String> {
    let output = serde_json::to_string_pretty(files).map_err(|error| error.to_string())?;
    println!("{output}");
    Ok(())
}

struct ParsedSource<'a> {
    source: &'a str,
    syntax: syn::File,
    lines: Vec<LineKind>,
    test_lines: BTreeSet<usize>,
}

impl<'a> ParsedSource<'a> {
    fn parse(source: &'a str) -> syn::Result<Self> {
        let syntax = syn::parse_file(source)?;
        let mut test_lines = TestLines::default();
        test_lines.visit_file(&syntax);

        Ok(Self {
            source,
            syntax,
            lines: classify_lines(source),
            test_lines: test_lines.lines,
        })
    }

    fn counts(&self) -> Counts {
        let mut counts = count_lines(&self.lines, &self.test_lines);
        counts.complexity = score(|visitor| visitor.visit_file(&self.syntax));
        counts
    }

    fn items(&self) -> Vec<ItemCounts> {
        self.module_items(&self.syntax.items)
    }

    fn module_items(&self, items: &[syn::Item]) -> Vec<ItemCounts> {
        items
            .iter()
            .filter(|item| !is_test_item(item))
            .filter_map(|item| self.item(item))
            .collect()
    }

    fn item(&self, item: &syn::Item) -> Option<ItemCounts> {
        use syn::Item;

        let (name, kind, anchor, children) = match item {
            Item::Const(item) => (
                item.ident.to_string(),
                "const",
                item.ident.span(),
                Vec::new(),
            ),
            Item::Enum(item) => (
                item.ident.to_string(),
                "enum",
                item.ident.span(),
                Vec::new(),
            ),
            Item::Fn(item) => (
                item.sig.ident.to_string(),
                "fn",
                item.sig.ident.span(),
                Vec::new(),
            ),
            Item::ForeignMod(item) => {
                let span = item.abi.span();
                let name = self.source_text(span.start(), span.end());
                (name, "extern", span, Vec::new())
            }
            Item::Impl(item) => {
                let start = item.impl_token.span.start();
                let name = self.source_text(start, item.self_ty.span().end());
                let methods = item
                    .items
                    .iter()
                    .filter_map(|member| match member {
                        syn::ImplItem::Fn(method) => Some(self.item_counts(
                            &method.sig.ident,
                            method.span(),
                            score(|visitor| visitor.visit_impl_item_fn(method)),
                        )),
                        _ => None,
                    })
                    .collect();
                (name, "impl", item.impl_token.span, methods)
            }
            Item::Macro(item) => {
                let name = match &item.ident {
                    Some(ident) => ident.to_string(),
                    None => format!("{}!", path_text(&item.mac.path)),
                };
                (name, "macro", item.mac.path.span(), Vec::new())
            }
            Item::Mod(item) => {
                let children = item
                    .content
                    .as_ref()
                    .map(|(_, items)| self.module_items(items))
                    .unwrap_or_default();
                (item.ident.to_string(), "mod", item.ident.span(), children)
            }
            Item::Static(item) => (
                item.ident.to_string(),
                "static",
                item.ident.span(),
                Vec::new(),
            ),
            Item::Struct(item) => (
                item.ident.to_string(),
                "struct",
                item.ident.span(),
                Vec::new(),
            ),
            Item::Trait(item) => {
                let methods = item
                    .items
                    .iter()
                    .filter_map(|member| match member {
                        syn::TraitItem::Fn(method) => Some(self.item_counts(
                            &method.sig.ident,
                            method.span(),
                            score(|visitor| visitor.visit_trait_item_fn(method)),
                        )),
                        _ => None,
                    })
                    .collect();
                (item.ident.to_string(), "trait", item.ident.span(), methods)
            }
            Item::TraitAlias(item) => (
                item.ident.to_string(),
                "trait",
                item.ident.span(),
                Vec::new(),
            ),
            Item::Type(item) => (
                item.ident.to_string(),
                "type",
                item.ident.span(),
                Vec::new(),
            ),
            Item::Union(item) => (
                item.ident.to_string(),
                "union",
                item.ident.span(),
                Vec::new(),
            ),
            _ => return None,
        };

        let (prod, doc) = self.count_extent(item.span());
        Some(ItemCounts {
            name,
            kind,
            line: anchor.start().line,
            prod,
            doc,
            cog: score(|visitor| visitor.visit_item(item)),
            items: children,
        })
    }

    fn item_counts(&self, ident: &Ident, extent: Span, cog: usize) -> ItemCounts {
        let (prod, doc) = self.count_extent(extent);
        ItemCounts {
            name: ident.to_string(),
            kind: "fn",
            line: ident.span().start().line,
            prod,
            doc,
            cog,
            items: Vec::new(),
        }
    }

    /// Counts the production code and comment lines in an item's extent. The
    /// extent already includes `///` doc comments; plain comments directly
    /// above the item, with no blank line in between, belong to it too.
    fn count_extent(&self, extent: Span) -> (usize, usize) {
        let is_leading_comment = |line: usize| {
            self.lines[line - 1] == LineKind::Comment && !self.test_lines.contains(&line)
        };

        let mut start = extent.start().line;
        while start > 1 && is_leading_comment(start - 1) {
            start -= 1;
        }

        let mut prod = 0;
        let mut doc = 0;
        for line in start..=extent.end().line {
            if self.test_lines.contains(&line) {
                continue;
            }
            match self.lines[line - 1] {
                LineKind::Code => prod += 1,
                LineKind::Comment => doc += 1,
                LineKind::Blank => {}
            }
        }

        (prod, doc)
    }

    /// The source between two positions, with runs of whitespace collapsed.
    fn source_text(&self, start: LineColumn, end: LineColumn) -> String {
        let mut text = String::new();
        for (number, line) in (start.line..=end.line).zip(self.source.lines().skip(start.line - 1))
        {
            let from = if number == start.line {
                start.column
            } else {
                0
            };
            let to = if number == end.line {
                end.column
            } else {
                usize::MAX
            };
            text.extend(line.chars().take(to).skip(from));
            text.push(' ');
        }
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }
}

fn score(visit: impl FnOnce(&mut Complexity)) -> usize {
    let mut complexity = Complexity::default();
    visit(&mut complexity);
    complexity.score
}

fn path_text(path: &syn::Path) -> String {
    path.segments
        .iter()
        .map(|segment| segment.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}

#[derive(Default)]
struct TestLines {
    lines: BTreeSet<usize>,
}

impl TestLines {
    fn mark_span(&mut self, span: Span) {
        self.lines.extend(span.start().line..=span.end().line);
    }
}

impl<'ast> Visit<'ast> for TestLines {
    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        if item.attrs.iter().any(is_test_attribute) {
            self.mark_span(item.span());
        }
        visit::visit_item_fn(self, item);
    }

    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        if item.attrs.iter().any(is_test_cfg) {
            self.mark_span(item.span());
        }
        visit::visit_item_mod(self, item);
    }
}

fn is_test_item(item: &syn::Item) -> bool {
    match item {
        syn::Item::Fn(item) => item.attrs.iter().any(is_test_attribute),
        syn::Item::Mod(item) => item.attrs.iter().any(is_test_cfg),
        _ => false,
    }
}

fn is_test_attribute(attribute: &Attribute) -> bool {
    attribute
        .path()
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "test")
}

fn is_test_cfg(attribute: &Attribute) -> bool {
    attribute.path().is_ident("cfg") && meta_contains_test(&attribute.meta)
}

fn meta_contains_test(meta: &Meta) -> bool {
    match meta {
        Meta::Path(path) => path.is_ident("test"),
        Meta::List(list) => list
            .parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
            .map(|metas| metas.iter().any(meta_contains_test))
            .unwrap_or(false),
        Meta::NameValue(_) => false,
    }
}

/// Cognitive complexity, following Campbell's specification: every construct
/// that breaks the linear flow of a function scores a point, and a construct
/// nested inside other flow-breaking constructs scores an extra point per
/// level of nesting. Declarations, plain blocks and `?` cost nothing.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LogicalOp {
    And,
    Or,
}

#[derive(Default)]
struct Complexity {
    score: usize,
    nesting: usize,
    depth: usize,
    logical: Option<LogicalOp>,
    enclosing: Vec<String>,
}

impl Complexity {
    fn increment(&mut self) {
        self.score += 1 + self.nesting;
    }

    fn nested(&mut self, visit: impl FnOnce(&mut Self)) {
        self.nesting += 1;
        visit(self);
        self.nesting -= 1;
    }

    fn visit_body(&mut self, name: &Ident, block: &Block) {
        // A function declared inside another function nests its contents.
        let is_nested = self.depth > 0;

        self.depth += 1;
        self.enclosing.push(name.to_string());
        if is_nested {
            self.nested(|visitor| visitor.visit_block(block));
        } else {
            self.visit_block(block);
        }
        self.enclosing.pop();
        self.depth -= 1;
    }

    fn visit_if(&mut self, node: &ExprIf, is_else_if: bool) {
        // `else if` continues an existing chain, so it escapes the nesting penalty.
        if is_else_if {
            self.score += 1;
        } else {
            self.increment();
        }

        self.visit_expr(&node.cond);
        self.nested(|visitor| visitor.visit_block(&node.then_branch));

        let Some((_, branch)) = &node.else_branch else {
            return;
        };

        if let Expr::If(next) = &**branch {
            self.visit_if(next, true);
        } else {
            self.score += 1;
            self.nested(|visitor| visitor.visit_expr(branch));
        }
    }

    fn is_recursive_call(&self, name: &Ident) -> bool {
        self.enclosing
            .last()
            .is_some_and(|enclosing| name == enclosing.as_str())
    }
}

impl<'ast> Visit<'ast> for Complexity {
    // Test code is reported in its own column, so it scores nothing here. The
    // checks mirror `TestLines` exactly, so the two columns stay in agreement.
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        if item.attrs.iter().any(is_test_cfg) {
            return;
        }
        visit::visit_item_mod(self, item);
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        if item.attrs.iter().any(is_test_attribute) {
            return;
        }
        self.visit_body(&item.sig.ident, &item.block);
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        self.visit_body(&item.sig.ident, &item.block);
    }

    fn visit_trait_item_fn(&mut self, item: &'ast syn::TraitItemFn) {
        if let Some(block) = &item.default {
            self.visit_body(&item.sig.ident, block);
        }
    }

    fn visit_expr(&mut self, node: &'ast Expr) {
        // Only a chain of logical operators continues a sequence; anything else
        // in between - parentheses, a call, a negation - starts a new one.
        let restore = match node {
            Expr::Binary(binary) if logical_op(&binary.op).is_some() => self.logical,
            _ => self.logical.take(),
        };

        visit::visit_expr(self, node);
        self.logical = restore;
    }

    fn visit_expr_binary(&mut self, node: &'ast syn::ExprBinary) {
        let Some(operator) = logical_op(&node.op) else {
            visit::visit_expr_binary(self, node);
            return;
        };

        // One point per sequence of the same operator: `a && b && c` scores
        // once, `a && b || c` scores twice.
        if self.logical != Some(operator) {
            self.score += 1;
        }

        let previous = self.logical.replace(operator);
        self.visit_expr(&node.left);
        self.visit_expr(&node.right);
        self.logical = previous;
    }

    fn visit_expr_if(&mut self, node: &'ast ExprIf) {
        self.visit_if(node, false);
    }

    fn visit_expr_match(&mut self, node: &'ast syn::ExprMatch) {
        self.increment();
        self.visit_expr(&node.expr);
        self.nested(|visitor| {
            for arm in &node.arms {
                visitor.visit_arm(arm);
            }
        });
    }

    fn visit_expr_loop(&mut self, node: &'ast syn::ExprLoop) {
        self.increment();
        self.nested(|visitor| visitor.visit_block(&node.body));
    }

    fn visit_expr_while(&mut self, node: &'ast syn::ExprWhile) {
        self.increment();
        self.visit_expr(&node.cond);
        self.nested(|visitor| visitor.visit_block(&node.body));
    }

    fn visit_expr_for_loop(&mut self, node: &'ast syn::ExprForLoop) {
        self.increment();
        self.visit_expr(&node.expr);
        self.nested(|visitor| visitor.visit_block(&node.body));
    }

    fn visit_expr_closure(&mut self, node: &'ast syn::ExprClosure) {
        // A closure scores nothing itself, it only nests what it contains.
        self.nested(|visitor| visitor.visit_expr(&node.body));
    }

    fn visit_expr_break(&mut self, node: &'ast syn::ExprBreak) {
        if node.label.is_some() {
            self.score += 1;
        }
        visit::visit_expr_break(self, node);
    }

    fn visit_expr_continue(&mut self, node: &'ast syn::ExprContinue) {
        if node.label.is_some() {
            self.score += 1;
        }
        visit::visit_expr_continue(self, node);
    }

    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        if let Expr::Path(path) = &*node.func
            && let Some(segment) = path.path.segments.last()
            && self.is_recursive_call(&segment.ident)
        {
            self.score += 1;
        }
        visit::visit_expr_call(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        if self.is_recursive_call(&node.method) {
            self.score += 1;
        }
        visit::visit_expr_method_call(self, node);
    }

    fn visit_local(&mut self, node: &'ast syn::Local) {
        let Some(init) = &node.init else {
            return;
        };

        self.visit_expr(&init.expr);

        // `let ... else` branches on the pattern failing to match.
        if let Some((_, diverge)) = &init.diverge {
            self.increment();
            self.nested(|visitor| visitor.visit_expr(diverge));
        }
    }
}

fn logical_op(operator: &syn::BinOp) -> Option<LogicalOp> {
    match operator {
        syn::BinOp::And(_) => Some(LogicalOp::And),
        syn::BinOp::Or(_) => Some(LogicalOp::Or),
        _ => None,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LineKind {
    Blank,
    Code,
    Comment,
}

fn classify_lines(source: &str) -> Vec<LineKind> {
    let mut scanner = LineScanner::default();
    source
        .lines()
        .map(|line| match scanner.scan_line(line) {
            (true, _) => LineKind::Code,
            (false, true) => LineKind::Comment,
            (false, false) => LineKind::Blank,
        })
        .collect()
}

fn count_lines(lines: &[LineKind], test_lines: &BTreeSet<usize>) -> Counts {
    let mut counts = Counts::default();

    for (index, kind) in lines.iter().enumerate() {
        let is_test = test_lines.contains(&(index + 1));

        match kind {
            LineKind::Code if is_test => counts.tests += 1,
            LineKind::Code => counts.code += 1,
            LineKind::Comment if !is_test => counts.comments += 1,
            LineKind::Comment | LineKind::Blank => {}
        }
    }

    counts
}

#[derive(Clone, Copy)]
enum LexState {
    Normal,
    LineComment,
    BlockComment(usize),
    String,
    Char,
    RawString(usize),
}

struct LineScanner {
    state: LexState,
}

impl Default for LineScanner {
    fn default() -> Self {
        Self {
            state: LexState::Normal,
        }
    }
}

impl LineScanner {
    fn scan_line(&mut self, line: &str) -> (bool, bool) {
        let chars: Vec<char> = line.chars().collect();
        let mut index = 0;
        let mut has_code = false;
        let mut has_comment = false;

        while index < chars.len() {
            match self.state {
                LexState::Normal => {
                    if let Some((next_index, hashes)) = raw_string_start(&chars, index) {
                        has_code = true;
                        self.state = LexState::RawString(hashes);
                        index = next_index;
                    } else if chars[index].is_whitespace() {
                        index += 1;
                    } else if chars[index] == '/' && chars.get(index + 1) == Some(&'/') {
                        has_comment = true;
                        self.state = LexState::LineComment;
                        break;
                    } else if chars[index] == '/' && chars.get(index + 1) == Some(&'*') {
                        has_comment = true;
                        self.state = LexState::BlockComment(1);
                        index += 2;
                    } else if chars[index] == '"' {
                        has_code = true;
                        self.state = LexState::String;
                        index += 1;
                    } else if chars[index] == '\'' && looks_like_char_literal(&chars, index) {
                        has_code = true;
                        self.state = LexState::Char;
                        index += 1;
                    } else {
                        has_code = true;
                        index += 1;
                    }
                }
                LexState::LineComment => {
                    has_comment = true;
                    break;
                }
                LexState::BlockComment(depth) => {
                    has_comment = true;
                    if chars[index] == '/' && chars.get(index + 1) == Some(&'*') {
                        self.state = LexState::BlockComment(depth + 1);
                        index += 2;
                    } else if chars[index] == '*' && chars.get(index + 1) == Some(&'/') {
                        if depth == 1 {
                            self.state = LexState::Normal;
                        } else {
                            self.state = LexState::BlockComment(depth - 1);
                        }
                        index += 2;
                    } else {
                        index += 1;
                    }
                }
                LexState::String => {
                    has_code = true;
                    if chars[index] == '\\' {
                        index += 2;
                    } else if chars[index] == '"' {
                        self.state = LexState::Normal;
                        index += 1;
                    } else {
                        index += 1;
                    }
                }
                LexState::Char => {
                    has_code = true;
                    if chars[index] == '\\' {
                        index += 2;
                    } else if chars[index] == '\'' {
                        self.state = LexState::Normal;
                        index += 1;
                    } else {
                        index += 1;
                    }
                }
                LexState::RawString(hashes) => {
                    has_code = true;
                    if chars[index] == '"' {
                        let mut closing_hashes = 0;
                        let mut closing_index = index + 1;
                        while chars.get(closing_index) == Some(&'#') {
                            closing_hashes += 1;
                            closing_index += 1;
                        }
                        if closing_hashes == hashes {
                            self.state = LexState::Normal;
                            index = closing_index;
                        } else {
                            index += 1;
                        }
                    } else {
                        index += 1;
                    }
                }
            }
        }

        if matches!(self.state, LexState::LineComment) {
            self.state = LexState::Normal;
        }

        (has_code, has_comment)
    }
}

fn raw_string_start(chars: &[char], index: usize) -> Option<(usize, usize)> {
    let prefix_end = match chars.get(index) {
        Some('r') => index + 1,
        Some('b' | 'c') if chars.get(index + 1) == Some(&'r') => index + 2,
        _ => return None,
    };

    let mut quote_index = prefix_end;
    while chars.get(quote_index) == Some(&'#') {
        quote_index += 1;
    }

    if chars.get(quote_index) == Some(&'"') {
        Some((quote_index + 1, quote_index - prefix_end))
    } else {
        None
    }
}

fn looks_like_char_literal(chars: &[char], index: usize) -> bool {
    let mut closing_index = index + 1;
    if chars.get(closing_index) == Some(&'\\') {
        closing_index += 2;
    } else {
        closing_index += 1;
    }
    chars.get(closing_index) == Some(&'\'')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count_source(source: &str) -> syn::Result<Counts> {
        Ok(ParsedSource::parse(source)?.counts())
    }

    fn items(source: &str) -> Vec<ItemCounts> {
        ParsedSource::parse(source).unwrap().items()
    }

    fn item(
        name: &str,
        kind: &'static str,
        line: usize,
        prod: usize,
        doc: usize,
        cog: usize,
    ) -> ItemCounts {
        ItemCounts {
            name: name.to_string(),
            kind,
            line,
            prod,
            doc,
            cog,
            items: Vec::new(),
        }
    }

    #[test]
    fn counts_code_comments_and_tests() {
        let source = r#"fn main() {
    println!("hello");
}

// A comment.
#[cfg(test)]
mod tests {
    #[test]
    fn works() {
        /* A test comment. */
        assert!(true);
    }
}
"#;

        assert_eq!(
            count_source(source).unwrap(),
            Counts {
                code: 3,
                comments: 1,
                tests: 7,
                complexity: 0,
            }
        );
    }

    fn complexity(source: &str) -> usize {
        count_source(source).unwrap().complexity
    }

    #[test]
    fn ignores_test_code() {
        let source = r#"fn prod(a: bool) {
    if a {
        println!("one");
    }
}

#[test]
fn free_test(a: bool) {
    if a {
        for _ in 0..3 {
            println!("ignored");
        }
    }
}

#[cfg(test)]
mod tests {
    fn helper(a: bool, b: bool) {
        if a && b {
            while a {
                println!("ignored");
            }
        }
    }
}
"#;

        assert_eq!(complexity(source), 1);
    }

    #[test]
    fn charges_a_penalty_for_each_level_of_nesting() {
        let source = r#"fn nested(a: bool, b: bool) {
    if a {
        if b {
            for _ in 0..3 {
                println!("deep");
            }
        }
    }
}
"#;

        assert_eq!(complexity(source), 6);
    }

    #[test]
    fn treats_else_if_as_a_flat_chain() {
        let source = r#"fn classify(n: i32) -> i32 {
    if n == 0 {
        0
    } else if n == 1 {
        1
    } else {
        2
    }
}
"#;

        assert_eq!(complexity(source), 3);
    }

    #[test]
    fn scores_each_sequence_of_logical_operators_once() {
        let source = r#"fn decide(a: bool, b: bool, c: bool, d: bool) -> bool {
    if a && b && c || d {
        return true;
    }

    a && (b && c)
}
"#;

        assert_eq!(complexity(source), 5);
    }

    #[test]
    fn nests_inside_closures_without_scoring_them() {
        let source = r#"fn each(values: Vec<Option<i32>>) {
    values.iter().for_each(|value| {
        match value {
            Some(_) => {}
            None => {}
        }
    });
}
"#;

        assert_eq!(complexity(source), 2);
    }

    #[test]
    fn scores_let_else_and_recursion() {
        let source = r#"fn walk(n: u32) -> u32 {
    let Some(next) = n.checked_sub(1) else {
        return 0;
    };

    walk(next)
}
"#;

        assert_eq!(complexity(source), 2);
    }

    #[test]
    fn scores_labelled_jumps_without_a_nesting_penalty() {
        let source = r#"fn search() {
    'outer: loop {
        loop {
            break 'outer;
        }
    }
}
"#;

        assert_eq!(complexity(source), 4);
    }

    #[test]
    fn ignores_comment_markers_in_strings() {
        let source = r##"fn main() {
    let line = "not a // comment";
    let block = r#"not a /* comment */"#;
}
"##;

        assert_eq!(
            count_source(source).unwrap(),
            Counts {
                code: 4,
                comments: 0,
                tests: 0,
                complexity: 0,
            }
        );
    }

    #[test]
    fn lists_items_without_tests() {
        let source = r#"use std::fmt;

/// A point.
struct Point {
    x: i32,
}

// Leading comment.
fn check(a: bool) -> bool {
    if a { true } else { false }
}

impl<'a> From<&'a str>
    for Point
where
    Point: Sized,
{
    fn from(_: &'a str) -> Self {
        // Inner comment.
        Point { x: 0 }
    }
}

mod inner {
    const LIMIT: usize = 3;

    #[cfg(test)]
    mod tests {}
}

#[test]
fn ignored() {}

#[cfg(test)]
mod tests {
    fn helper() {}
}
"#;

        assert_eq!(
            items(source),
            vec![
                item("Point", "struct", 4, 3, 1, 0),
                item("check", "fn", 9, 3, 1, 2),
                ItemCounts {
                    items: vec![item("from", "fn", 18, 3, 1, 0)],
                    ..item("impl<'a> From<&'a str> for Point", "impl", 13, 9, 1, 0)
                },
                ItemCounts {
                    items: vec![item("LIMIT", "const", 25, 1, 0, 0)],
                    ..item("inner", "mod", 24, 3, 0, 0)
                },
            ]
        );
    }
}
