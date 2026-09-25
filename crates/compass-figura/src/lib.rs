//! The figura IDL compiler, TypeScript half.
//!
//! `figura/*.fig` declares the JSON-RPC protocols the extension runtime speaks:
//! structs, enums, and services of methods and events. This generates the
//! TypeScript bindings the runtime and `@vicinae/api` are built against. It is
//! a port of the C++ `figura` (`src/lib/figura`, removed with the C++ engine,
//! ADR-0021), and its output is byte-for-byte what that compiler produced, so
//! the committed bindings did not change when the generator did.
//!
//! The C++ compiler's other two back ends generated Qt and glaze servers for
//! the C++ engine. The Rust engine hand-writes its side and checks it against
//! the IDL in its own tests, so only TypeScript is ported.
//!
//! The lexer and parser keep the original's grammar, quirks included, because
//! the committed `.fig` files are written to it. The one deliberate difference
//! is that input the original would loop on forever (a stray token at the top
//! level) is an error here.

/// The protocols the build commits, as `(IDL, side, output)`, relative to the
/// repository root. `make figen` writes them and a test keeps them current.
pub const COMMITTED: &[(&str, Side, &str)] = &[
    (
        "figura/manager.fig",
        Side::Server,
        "src/typescript/extension-manager/src/proto/manager.ts",
    ),
    (
        "figura/manager-extension.fig",
        Side::Client,
        "src/typescript/extension-manager/src/proto/manager-extension.ts",
    ),
    (
        "figura/manager-extension.fig",
        Side::Server,
        "src/typescript/extension-manager/src/proto/extension-manager.ts",
    ),
    (
        "figura/tsapi.fig",
        Side::Client,
        "src/typescript/extension-manager/src/proto/api.ts",
    ),
    (
        "figura/tsapi.fig",
        Side::Client,
        "src/typescript/api/src/api/proto/api.ts",
    ),
    (
        "figura/ipc.fig",
        Side::Client,
        "src/typescript/api/src/proto/ipc.ts",
    ),
];

/// Which end of a protocol to generate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    /// Typed calls and event subscriptions over a transport.
    Client,
    /// Abstract services to implement, and the router that dispatches to them.
    Server,
}

/// Parses `source` and generates `side` of it in TypeScript.
///
/// # Errors
///
/// A parse error, with the line and column it stopped at.
pub fn compile(source: &str, side: Side) -> Result<String, String> {
    let tree = parse(source)?;
    Ok(match side {
        Side::Client => typescript::client(&tree),
        Side::Server => typescript::server(&tree),
    })
}

/// A scalar the IDL knows by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Primitive {
    Void,
    Boolean,
    String,
    Int,
    UInt,
    Double,
    Any,
}

/// A primitive, or a struct or enum declared earlier in the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeRef<'a> {
    Primitive(Primitive),
    Declared(&'a str),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Type<'a> {
    pub base: TypeRef<'a>,
    pub is_array: bool,
    pub is_optional: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field<'a> {
    pub name: &'a str,
    pub ty: Type<'a>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Method<'a> {
    pub name: &'a str,
    pub params: Vec<Field<'a>>,
    pub returns: Type<'a>,
    pub is_async: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Event<'a> {
    pub name: &'a str,
    pub params: Vec<Field<'a>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Service<'a> {
    pub name: &'a str,
    pub methods: Vec<Method<'a>>,
    pub events: Vec<Event<'a>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Struct<'a> {
    pub name: &'a str,
    pub fields: Vec<Field<'a>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Enum<'a> {
    pub name: &'a str,
    pub values: Vec<&'a str>,
}

/// A parsed `.fig` file, each list in declaration order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tree<'a> {
    pub services: Vec<Service<'a>>,
    pub structs: Vec<Struct<'a>>,
    pub enums: Vec<Enum<'a>>,
}

/// Parses a `.fig` file.
///
/// # Errors
///
/// A parse error, with the line and column it stopped at.
pub fn parse(source: &str) -> Result<Tree<'_>, String> {
    let mut parser = Parser {
        lexer: Lexer::new(source),
        tree: Tree::default(),
    };
    match parser.parse() {
        Ok(()) => Ok(parser.tree),
        Err(reason) => {
            let (line, column) = parser.lexer.position();
            let context = source.lines().nth(line).unwrap_or_default();
            Err(format!(
                "Error: {reason} (L {}:{column})\n{context}",
                line + 1
            ))
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Identifier,
    Service,
    Method,
    Async,
    Enum,
    Struct,
    Event,
    Colon,
    Arrow,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Semicolon,
    QuestionMark,
}

#[derive(Clone, Copy, Debug)]
struct Token<'a> {
    kind: Kind,
    data: &'a str,
}

fn classify(word: &str) -> Kind {
    match word {
        "struct" => Kind::Struct,
        "enum" => Kind::Enum,
        "service" => Kind::Service,
        "event" => Kind::Event,
        "async" => Kind::Async,
        "fn" => Kind::Method,
        "=>" => Kind::Arrow,
        "(" => Kind::LParen,
        ")" => Kind::RParen,
        "{" => Kind::LBrace,
        "}" => Kind::RBrace,
        ":" => Kind::Colon,
        "," => Kind::Comma,
        ";" => Kind::Semicolon,
        _ => Kind::Identifier,
    }
}

fn is_identifier_byte(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// C's `isspace` in the C locale, which includes the vertical tab that
/// `u8::is_ascii_whitespace` leaves out.
fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

struct Lexer<'a> {
    data: &'a str,
    cursor: usize,
    current: Option<Token<'a>>,
}

impl<'a> Lexer<'a> {
    fn new(data: &'a str) -> Self {
        Self {
            data,
            cursor: 0,
            current: None,
        }
    }

    fn peek(&self) -> Option<Token<'a>> {
        self.current
    }

    fn advance(&mut self) -> Option<Token<'a>> {
        self.current = self.scan();
        self.current
    }

    /// The zero-based line and column of the cursor.
    fn position(&self) -> (usize, usize) {
        let consumed = &self.data.as_bytes()[..self.cursor.min(self.data.len())];
        let line = consumed.iter().filter(|&&c| c == b'\n').count();
        let column = consumed.iter().rev().take_while(|&&c| c != b'\n').count();
        (line, column)
    }

    fn word(&self, start: usize) -> Token<'a> {
        let data = self.data.get(start..self.cursor).unwrap_or_default();
        Token {
            kind: classify(data),
            data,
        }
    }

    fn scan(&mut self) -> Option<Token<'a>> {
        enum State {
            Reset,
            Operator,
            Word,
            ForwardSlash,
            Comment,
        }

        let bytes = self.data.as_bytes();
        let mut state = State::Reset;
        let mut start = 0;

        while let Some(&c) = bytes.get(self.cursor) {
            match state {
                State::Reset => {
                    if is_identifier_byte(c) {
                        state = State::Word;
                        start = self.cursor;
                    } else if c == b'/' {
                        state = State::ForwardSlash;
                    } else if !is_space(c) {
                        let single = match c {
                            b'{' => Some(Kind::LBrace),
                            b'}' => Some(Kind::RBrace),
                            b'(' => Some(Kind::LParen),
                            b')' => Some(Kind::RParen),
                            b'[' => Some(Kind::LBracket),
                            b']' => Some(Kind::RBracket),
                            b'?' => Some(Kind::QuestionMark),
                            _ => None,
                        };
                        if let Some(kind) = single {
                            self.cursor += 1;
                            return Some(Token { kind, data: "" });
                        }
                        state = State::Operator;
                        start = self.cursor;
                    }
                }
                State::ForwardSlash => {
                    if c == b'/' {
                        state = State::Comment;
                    }
                }
                State::Comment => {
                    if c == b'\n' {
                        state = State::Reset;
                    }
                }
                State::Operator => {
                    if (c.is_ascii_alphanumeric() || is_space(c)) && start != self.cursor {
                        return Some(self.word(start));
                    }
                }
                State::Word => {
                    if !is_identifier_byte(c) && start != self.cursor {
                        return Some(self.word(start));
                    }
                }
            }
            self.cursor += 1;
        }

        None
    }
}

struct Parser<'a> {
    lexer: Lexer<'a>,
    tree: Tree<'a>,
}

type Parsed<T> = Result<T, String>;

impl<'a> Parser<'a> {
    fn parse(&mut self) -> Parsed<()> {
        self.lexer.advance();

        while let Some(token) = self.lexer.peek() {
            match token.kind {
                Kind::Struct => {
                    let parsed = self.parse_struct()?;
                    self.tree.structs.push(parsed);
                }
                Kind::Service => {
                    let parsed = self.parse_service()?;
                    self.tree.services.push(parsed);
                }
                Kind::Enum => {
                    let parsed = self.parse_enum()?;
                    self.tree.enums.push(parsed);
                }
                Kind::Semicolon => {
                    self.lexer.advance();
                }
                other => {
                    return Err(format!(
                        "expected struct, enum or service at the top level (got {other:?})"
                    ));
                }
            }
        }

        Ok(())
    }

    fn expect(&mut self, kind: Kind, reason: &str) -> Parsed<Token<'a>> {
        match self.lexer.peek() {
            Some(token) if token.kind == kind => {
                self.lexer.advance();
                Ok(token)
            }
            _ => Err(reason.to_owned()),
        }
    }

    fn peek_unless(&self, kind: Kind) -> Parsed<Option<Token<'a>>> {
        let token = self.lexer.peek().ok_or("No more token")?;
        Ok((token.kind != kind).then_some(token))
    }

    fn peek_is(&self, kind: Kind) -> bool {
        self.lexer.peek().is_some_and(|token| token.kind == kind)
    }

    fn resolve(&self, name: &'a str) -> Option<TypeRef<'a>> {
        let primitive = match name {
            "string" => Primitive::String,
            "int" => Primitive::Int,
            "uint" => Primitive::UInt,
            "double" => Primitive::Double,
            "boolean" | "bool" => Primitive::Boolean,
            "void" => Primitive::Void,
            "any" => Primitive::Any,
            _ => {
                let declared = self.tree.structs.iter().any(|s| s.name == name)
                    || self.tree.enums.iter().any(|e| e.name == name);
                return declared.then_some(TypeRef::Declared(name));
            }
        };
        Some(TypeRef::Primitive(primitive))
    }

    fn parse_type(&mut self) -> Parsed<Type<'a>> {
        let token = self
            .lexer
            .peek()
            .filter(|token| token.kind == Kind::Identifier)
            .ok_or("Expected type identifier")?;
        let base = self
            .resolve(token.data)
            .ok_or_else(|| format!("{} is not a valid type", token.data))?;
        self.lexer.advance();

        let mut ty = Type {
            base,
            is_array: false,
            is_optional: false,
        };
        if self.peek_is(Kind::LBracket) {
            self.lexer.advance();
            if self.peek_is(Kind::RBracket) {
                ty.is_array = true;
                self.lexer.advance();
            }
        }
        Ok(ty)
    }

    /// `<name>[?]: <type>`
    fn parse_field(&mut self) -> Parsed<Field<'a>> {
        let name = self.expect(Kind::Identifier, "assertPeak failed")?.data;
        let optional = self.peek_is(Kind::QuestionMark);
        if optional {
            self.lexer.advance();
        }
        self.expect(Kind::Colon, "expected colon after struct field name")?;
        let mut ty = self.parse_type()?;
        ty.is_optional = optional;
        Ok(Field { name, ty })
    }

    fn parse_parameters(&mut self) -> Parsed<Vec<Field<'a>>> {
        let mut params = Vec::new();
        self.expect(Kind::LParen, "expected ( to start method parameters")?;

        while let Some(token) = self.lexer.peek() {
            match token.kind {
                Kind::RParen => break,
                Kind::Comma => {
                    self.lexer.advance();
                }
                _ => params.push(self.parse_field()?),
            }
        }

        self.expect(
            Kind::RParen,
            "rparen at the end of parameter list was expected",
        )?;
        Ok(params)
    }

    fn parse_service(&mut self) -> Parsed<Service<'a>> {
        self.expect(Kind::Service, "Expected service token")?;
        let name = self
            .expect(
                Kind::Identifier,
                "expected identifier after service keyword",
            )?
            .data;
        self.expect(Kind::LBrace, "expected lbrace to define service")?;

        let mut service = Service {
            name,
            methods: Vec::new(),
            events: Vec::new(),
        };
        while let Some(token) = self.peek_unless(Kind::RBrace)? {
            match token.kind {
                Kind::Method | Kind::Async => service.methods.push(self.parse_method()?),
                Kind::Event => service.events.push(self.parse_event()?),
                _ => return Err(r#"Expected "fn" or "event" keyword in service"#.to_owned()),
            }
        }

        self.expect(Kind::RBrace, "rbrace expected to close service")?;
        Ok(service)
    }

    fn parse_method(&mut self) -> Parsed<Method<'a>> {
        let is_async = self.peek_is(Kind::Async);
        if is_async {
            self.lexer.advance();
        }
        self.expect(Kind::Method, "expected fn keyword before method")?;
        let name = self
            .expect(Kind::Identifier, "Expected identifier after \"fn\"")?
            .data;
        let params = self.parse_parameters()?;
        self.expect(Kind::Arrow, "Expected => <return_type> for service method")?;
        let returns = self.parse_type()?;
        self.expect(
            Kind::Semicolon,
            "expected semicolon at the end of service method",
        )?;
        Ok(Method {
            name,
            params,
            returns,
            is_async,
        })
    }

    fn parse_event(&mut self) -> Parsed<Event<'a>> {
        self.expect(Kind::Event, "Expected event")?;
        let name = self
            .expect(Kind::Identifier, "Expected identifier after \"event\"")?
            .data;
        let params = self.parse_parameters()?;
        self.expect(
            Kind::Semicolon,
            "expected semicolon at the end of event method",
        )?;
        Ok(Event { name, params })
    }

    fn parse_enum(&mut self) -> Parsed<Enum<'a>> {
        self.expect(Kind::Enum, "expected enum kw")?;
        let name = self
            .expect(Kind::Identifier, "expected identifier after enum keyword")?
            .data;
        self.expect(Kind::LBrace, "expected lbrace")?;

        let mut values = Vec::new();
        while let Some(token) = self.peek_unless(Kind::RBrace)? {
            match token.kind {
                Kind::Comma => {}
                Kind::Identifier => values.push(token.data),
                _ => return Err("Expected identifier in enum".to_owned()),
            }
            self.lexer.advance();
        }

        self.expect(Kind::RBrace, "assertPeak failed")?;
        Ok(Enum { name, values })
    }

    fn parse_struct(&mut self) -> Parsed<Struct<'a>> {
        self.expect(Kind::Struct, "Expected struct")?;
        let name = self
            .expect(Kind::Identifier, "expected identifier after struct keyword")?
            .data;
        self.expect(Kind::LBrace, "expected lbrace after struct name")?;

        let mut fields = Vec::new();
        while self.peek_unless(Kind::RBrace)?.is_some() {
            fields.push(self.parse_field()?);
            self.expect(
                Kind::Semicolon,
                "expected semicolon at the end of struct field",
            )?;
        }

        self.expect(Kind::RBrace, "assertPeak failed")?;
        Ok(Struct { name, fields })
    }
}

pub mod typescript {
    //! The TypeScript back end.

    use std::fmt::Write as _;

    use super::{Enum, Event, Field, Primitive, Service, Struct, Tree, Type, TypeRef};

    const COMMON: &str = include_str!("typescript/common.ts.in");
    const CLIENT_BUS: &str = include_str!("typescript/client-bus.ts.in");
    const CLIENT_ROUTE: &str = include_str!("typescript/client-route.ts.in");
    const SERVER_BOILERPLATE: &str = include_str!("typescript/server-boilerplate.ts.in");
    const SERVER_ROUTE: &str = include_str!("typescript/server-route.ts.in");

    /// The JSON-RPC method name a call or event travels under.
    #[must_use]
    pub fn wire_name(service: &str, method: &str) -> String {
        format!("{service}/{method}")
    }

    fn type_name<'a>(ty: &Type<'a>) -> &'a str {
        match ty.base {
            TypeRef::Declared(name) => name,
            TypeRef::Primitive(Primitive::Void) => "void",
            TypeRef::Primitive(Primitive::Boolean) => "boolean",
            TypeRef::Primitive(Primitive::Int | Primitive::UInt | Primitive::Double) => "number",
            TypeRef::Primitive(Primitive::String) => "string",
            TypeRef::Primitive(Primitive::Any) => "any",
        }
    }

    fn signature(ty: &Type<'_>) -> String {
        let mut out = type_name(ty).to_owned();
        if ty.is_array {
            out.push_str("[]");
        }
        out
    }

    fn joined<T>(items: &[T], each: impl Fn(&T) -> String) -> String {
        items.iter().map(each).collect::<Vec<_>>().join(", ")
    }

    fn struct_type(s: &Struct<'_>) -> String {
        let mut out = format!("export type {} = {{\n", s.name);
        for field in &s.fields {
            let optional = if field.ty.is_optional { "?" } else { "" };
            let _ = writeln!(out, "\t{}{optional}: {};", field.name, signature(&field.ty));
        }
        out.push('}');
        out
    }

    fn enum_type(e: &Enum<'_>) -> String {
        let values = e
            .values
            .iter()
            .map(|value| format!("'{value}'"))
            .collect::<Vec<_>>()
            .join(" | ");
        format!("export type {} = {values};", e.name)
    }

    fn types(out: &mut String, tree: &Tree<'_>) {
        for e in &tree.enums {
            out.push_str(&enum_type(e));
            out.push_str("\n\n");
        }
        for s in &tree.structs {
            out.push_str(&struct_type(s));
            out.push_str("\n\n");
        }
    }

    /// `name: type` for each parameter, without the array suffix: the C++
    /// generator wrote server signatures this way and the committed bindings
    /// carry it.
    fn plain_parameters(params: &[Field<'_>]) -> String {
        joined(params, |p| format!("{}: {}", p.name, type_name(&p.ty)))
    }

    fn names(params: &[Field<'_>], prefix: &str) -> String {
        joined(params, |p| format!("{prefix}{}", p.name))
    }

    fn event_handler(event: &Event<'_>) -> String {
        let params = joined(&event.params, |p| {
            format!("{}: {}", p.name, signature(&p.ty))
        });
        format!("({params}) => void")
    }

    fn client_service(s: &Service<'_>) -> String {
        let mut out = format!("class {}Service {{\n", s.name);
        out.push_str("\tconstructor(private readonly transport: RpcTransport) {}\n\n");

        for method in &s.methods {
            let params = joined(&method.params, |p| {
                let optional = if p.ty.is_optional { "?" } else { "" };
                format!("{}{optional}: {}", p.name, signature(&p.ty))
            });
            let _ = write!(
                out,
                "\t{}({params}): Promise<{}> {{\n\t\treturn this.transport.request(\"{}\", {{ {}}});\t\n\t}}\n\n",
                method.name,
                signature(&method.returns),
                wire_name(s.name, method.name),
                names(&method.params, ""),
            );
        }

        for event in &s.events {
            let _ = write!(
                out,
                "\t{}(handler: {}): EventSubscription {{\n\t\treturn this.transport.subscribe(\"{}\", (msg) => handler({}))\n\t}}\n",
                event.name,
                event_handler(event),
                wire_name(s.name, event.name),
                names(&event.params, "msg."),
            );
        }

        out.push('}');
        out
    }

    /// The typed client: one service object per `service`, over a transport
    /// that matches replies to requests and fans events out to subscribers.
    #[must_use]
    pub fn client(tree: &Tree<'_>) -> String {
        let mut out = format!("{COMMON}{CLIENT_BUS}\n");
        types(&mut out, tree);

        for s in &tree.services {
            out.push_str(&client_service(s));
            out.push_str("\n\n");
        }

        out.push_str("export class Client {\n");
        out.push_str("\tconstructor(private readonly transport: RpcTransport) {\n");
        for s in &tree.services {
            let _ = writeln!(
                out,
                "\t\tthis.{} = new {}Service(this.transport);",
                s.name, s.name
            );
        }
        out.push_str("\t}\n");
        out.push_str(CLIENT_ROUTE);
        for s in &tree.services {
            let _ = writeln!(out, "\t{}: {}Service;", s.name, s.name);
        }
        out.push_str("\n}\n");
        out
    }

    fn is_void(ty: &Type<'_>) -> bool {
        ty.base == TypeRef::Primitive(Primitive::Void)
    }

    fn server_cases(tree: &Tree<'_>) -> String {
        let mut out = String::new();
        for s in &tree.services {
            for m in &s.methods {
                let reply = if is_void(&m.returns) {
                    ".then(() => this.rpc.reply(id, null))"
                } else {
                    ".then((r) => this.rpc.reply(id, r))"
                };
                let _ = writeln!(
                    out,
                    "case \"{}\":\treturn this.{}.{}({}){reply}",
                    wire_name(s.name, m.name),
                    s.name,
                    m.name,
                    names(&m.params, "msg.params."),
                );
            }
        }
        out.push_str("default: throw new Error(`No handler for method ${msg.method}`)");
        out
    }

    /// The server: an abstract class per service with methods, to implement,
    /// and a `Server` whose `route` dispatches requests to them.
    #[must_use]
    pub fn server(tree: &Tree<'_>) -> String {
        let mut out = format!("{COMMON}{SERVER_BOILERPLATE}");
        types(&mut out, tree);

        for service in tree.services.iter().filter(|s| !s.methods.is_empty()) {
            let _ = writeln!(out, "export abstract class {}Service {{", service.name);
            out.push_str("\tconstructor(private readonly rpc: RpcTransport) {}\n");
            for m in &service.methods {
                let _ = writeln!(
                    out,
                    "\tabstract {}({}): Promise <{}>;",
                    m.name,
                    plain_parameters(&m.params),
                    type_name(&m.returns),
                );
            }
            for event in &service.events {
                let _ = write!(
                    out,
                    "emit_{}({}) {{\n\tthis.rpc.emit(\"{}\", {{ {}}});\n}}",
                    event.name,
                    plain_parameters(&event.params),
                    wire_name(service.name, event.name),
                    names(&event.params, ""),
                );
            }
            out.push_str("\n}\n");
        }

        out.push_str("export class Server {\n");
        out.push_str("\tconstructor(private readonly rpc: RpcTransport");
        for service in tree.services.iter().filter(|s| !s.methods.is_empty()) {
            let _ = write!(out, ", readonly {}: {}Service", service.name, service.name);
        }
        out.push_str(") {\n");
        out.push_str("}\n\n");
        out.push_str(SERVER_ROUTE);
        out.push_str(&server_cases(tree));
        out.push_str("\n}\n}\n}\n}");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_small_protocol_generates_both_sides() {
        let fig = "
            // comment
            enum Mode { Fast, Slow }
            struct Item { id: string; tags?: string[]; mode: Mode; }
            service Things {
                fn get(id: string, all?: boolean) => Item[];
                fn drop(id: string) => void;
                event changed(item: Item, count: int);
            }
        ";
        let tree = parse(fig).expect("parses");
        assert_eq!(tree.enums[0].values, ["Fast", "Slow"]);
        assert!(tree.structs[0].fields[1].ty.is_optional);
        assert!(tree.structs[0].fields[1].ty.is_array);

        let client = typescript::client(&tree);
        assert!(client.contains("export type Mode = 'Fast' | 'Slow';"));
        assert!(client.contains("\ttags?: string[];\n"));
        assert!(client.contains(
            "\tget(id: string, all?: boolean): Promise<Item[]> {\n\t\treturn this.transport.request(\"Things/get\", { id, all});\t\n\t}\n"
        ));
        assert!(client.contains(
            "\treturn this.transport.subscribe(\"Things/changed\", (msg) => handler(msg.item, msg.count))"
        ));

        let server = typescript::server(&tree);
        assert!(server.contains("\tabstract get(id: string, all: boolean): Promise <Item>;\n"));
        assert!(server.contains("emit_changed(item: Item, count: number) {"));
        assert!(server.contains(
            "case \"Things/drop\":\treturn this.Things.drop(msg.params.id).then(() => this.rpc.reply(id, null))\n"
        ));
    }

    #[test]
    fn an_undeclared_type_is_an_error_with_a_position() {
        let err = parse("struct A {\n  b: Missing;\n}").expect_err("Missing is not declared");
        assert!(err.contains("Missing is not a valid type"), "{err}");
        assert!(err.contains("(L 2:"), "{err}");
    }

    #[test]
    fn a_stray_top_level_token_is_an_error_not_a_hang() {
        assert!(parse("oops;").is_err());
    }
}
