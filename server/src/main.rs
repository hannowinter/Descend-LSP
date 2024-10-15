use std::{collections::HashMap, fmt::format, io::{BufRead, Write}, str::FromStr};

use serde::{Deserialize, Serialize};

pub mod structures;
use serde_json::Value;
use structures::*;

use router_macro::route;

// Raw message according to the LSP Base Protocol, consisting of a HTTP-like header and content part:
//
// Content-Length: 123
// Content-Type: application/vscode-jsonrpc; charset=utf-8
// 
// { ... }
//
// Note that the Content-Type field is optional and \r\n is used for the line breaks.
#[derive(Debug)]
pub struct RawMessage {
    pub content_length: usize,
    pub content_type: String,
    pub content: String
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponseError {
    pub code: i32,
    pub message: String,
    pub data: Option<Value>
}

#[allow(dead_code)]
impl ResponseError {
    const PARSE_ERROR: i32 = -32700;
    const INVALID_REQUEST: i32 = -32600;
    const METHOD_NOT_FOUND: i32 = -32601;
    const INVALID_PARAMS: i32 = -32602;
    const INTERNAL_ERROR: i32 = -32603;

    const SERVER_NOT_INITIALIZED: i32 = -32002;
    const UNKNOWN_ERROR_CODE: i32 = -32001;
    const REQUEST_FAILED: i32 = -32802;
    const SERVER_CANCELLED: i32 = -32802;
    const CONTENT_MODIFIED: i32 = -32801;
    const REQUEST_CANCELLED: i32 = -32800;
}

// Request message according to the LSP
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestMessage {
    pub jsonrpc: String,
    pub id: Id,
    pub method: String,
    pub params: Value
}

// Response message according to the LSP
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponseMessage {
    pub jsonrpc: String,
    pub id: Id,
    pub result: Option<Value>,
    pub error: Option<ResponseError>
}

// Notification message according to the LSP
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationMessage {
    pub jsonrpc: String,
    pub method: String,
    pub params: Value
}

impl ResponseMessage {
    fn error(id: Id, code: i32, message: String) -> ResponseMessage {
        ResponseMessage {
            jsonrpc: String::from("2.0"),
            id,
            result: None,
            error: Some(ResponseError {
                code,
                message,
                data: None
            })
        }
    }
}

// Message base, the deserializer will pick the right one
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", untagged)]
pub enum Message {
    Request(RequestMessage),
    Response(ResponseMessage),
    Notification(NotificationMessage)
}

// Result of "initialize" request
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub capabilities: ServerCapabilities,
    pub server_info: ServerInfo
}

fn bind_by_ref<T, R>(mut f: impl FnMut(&T) -> R) -> impl FnMut(T) -> R {
    move |x| f(&x)
}

impl RawMessage {
    // Reads from the specified buffer to create a new raw message
    fn read(buf: &mut impl BufRead) -> Result<RawMessage, String> {
        let mut message: RawMessage = RawMessage{ content_length: 0, content_type: String::new(), content: String::new() };
        let mut read_any = false;

        // repeatedly read all the header fields, which are of the form "X: Y\r\n"
        // a blank line "\r\n" indicates the end of the header and the begin of the content
        loop {
            let mut header_field = String::new();
            buf.read_line(&mut header_field).map_err(bind_by_ref(std::io::Error::to_string))?;
            if header_field.trim().is_empty() {
                if read_any {
                    break;
                } else {
                    continue;
                }
            }
            let mut parts = header_field.split(':');
            let header_field_name = parts.next().ok_or("Syntax error: Header field needs to be of the form \"X: Y\r\n\"!")?.trim();
            let header_field_value = parts.next().ok_or("Syntax error: Header field needs to be of the form \"X: Y\r\n\"!")?.trim();

            match header_field_name {
                "Content-Length" => message.content_length = header_field_value.parse().map_err(bind_by_ref(<usize as FromStr>::Err::to_string))?,
                "Content-Type" => message.content_type = header_field_value.to_string(),
                &_ => return Err(format(format_args!("Unexpected header field \"{header_field_name}\"!")))
            };
            read_any = true;
        }

        let mut content = vec![0u8; message.content_length];
        buf.read_exact(&mut content).expect("Error while reading from buffer!");
        message.content = String::from_utf8(content).expect("Error while converting content bytes to UTF8!");
        
        Ok(message)
    }

    // Constructs a raw message from its content part
    fn from(content: serde_json::Value) -> RawMessage {
        let content = content.to_string();
        RawMessage{ content_length: content.len(), content_type: String::new(), content }
    }

    // Writes a raw message to the specified buffer
    fn write(&self, buf: &mut impl Write) -> Result<(), String> {
        let content_length = self.content_length;
        let content_type = &self.content_type;

        buf.write_fmt(format_args!("Content-Length: {content_length}\r\n")).map_err(bind_by_ref(std::io::Error::to_string))?;
        if !content_type.is_empty() {
            buf.write_fmt(format_args!("Content-Type: {content_type}\r\n")).map_err(bind_by_ref(std::io::Error::to_string))?;
        }
        buf.write_fmt(format_args!("\r\n")).map_err(bind_by_ref(std::io::Error::to_string))?;
        let content = self.content.to_string();
        buf.write_fmt(format_args!("{content}")).map_err(bind_by_ref(std::io::Error::to_string))?;
        Ok(())
    }
}

impl Message {
    // Construct a message from a raw message
    fn from_raw(raw: &RawMessage) -> Result<Message, String> {
        // serde will automatically pick the correct message type
        serde_json::from_str::<Message>(&raw.content).map_err(bind_by_ref(serde_json::Error::to_string))
    }
}

// Represents a text document as an array of lines
#[derive(Debug, PartialEq)]
pub struct TextDocument {
    pub lines: Vec<String>
}

// todo UTF beachten
impl TextDocument {
    // Erases the specified range
    fn erase(&mut self, range: &Range) {
        let mut c = 0usize;
        let c_total = (range.end.line - range.start.line + 1) as usize;
        
        while c < c_total {
            let first = c == 0;
            let last = c == c_total - 1;

            if first && last { // range only spans a single line
                self.lines[range.start.line as usize].replace_range((range.start.character as usize)..(range.end.character as usize), "");
            } else if first {
                self.lines[range.start.line as usize].replace_range((range.start.character as usize).., "");
            } else if !first && !last {
                self.lines.remove(range.start.line as usize + 1); // be careful with the index on the collection we are currently removing elements from
            } else if last { // remove last line and append its tail to the first line
                let line = self.lines.remove(range.start.line as usize + 1);
                let line_tail = &line[(range.end.character as usize)..];
                self.lines[range.start.line as usize].push_str(line_tail);
            } 

            c += 1;
        }
    }

    // Inserts the specified text at specified position
    fn insert(&mut self, position: &Position, text: &str) {
        let text_lines = text.split("\r\n");
        let lines_count = text_lines.clone().count();

        let mut i = 0usize;
        for text_line in text_lines {
            let first = i == 0;
            let last = i == lines_count - 1;

            if first && last { // text only has a single line
                self.lines[position.line as usize].insert_str(position.character as usize, text_line);
            } else if first { // break document line at specified position and append first text line to it
                let mut line_head = self.lines.remove(position.line as usize);
                let line_tail = line_head.split_off(position.character as usize);

                self.lines.insert(position.line as usize, line_head);
                self.lines[position.line as usize].push_str(text_line);
                self.lines.insert(position.line as usize + 1, line_tail);
            } else if !first && !last {
                self.lines.insert(position.line as usize + i, String::from(text_line));
            } else if last {
                self.lines[position.line as usize + i].insert_str(0usize, text_line);
            }

            i += 1;
        }
    }

    // Replaces specified range with specified text
    fn edit(&mut self, range: &Range, text: &str) {
        self.erase(&range);
        self.insert(&range.start, text);
    }
}

#[test]
fn test_erase() {
    let mut content = TextDocument {
        lines: vec![
            String::from("01234"),
            String::from("56789"),
            String::from("abcde")
        ]
    };
    let match1 = TextDocument {
        lines: vec![String::from("012de")]
    };
    let match2 = TextDocument {
        lines: vec![String::from("01e")]
    };
    
    content.erase(&Range {
        start: Position {
            line: 0,
            character: 3
        },
        end: Position {
            line: 2,
            character: 3
        }
    });
    assert_eq!(content, match1);
    content.erase(&Range {
        start: Position {
            line: 0,
            character: 2
        },
        end: Position {
            line: 0,
            character: 4
        }
    });
    assert_eq!(content, match2);
}

#[test]
fn test_insert() {
    let mut content = TextDocument {
        lines: vec![String::from("01e")]
    };
    let match1 = TextDocument {
        lines: vec![String::from("012de")]
    };
    let match2 = TextDocument {
        lines: vec![
            String::from("01234"),
            String::from("56789"),
            String::from("abcde")
        ]
    };
    
    content.insert(&Position {
        line: 0,
        character: 2
    }, "2d");
    assert_eq!(content, match1);
    content.insert(&Position {
        line: 0,
        character: 3
    }, "34\r\n56789\r\nabc");
    assert_eq!(content, match2);
}

// All requests and notifications get routed to their corresponding handler function
#[route]
pub trait Router {
    fn state(&mut self) -> &mut State;

    #[route("initialize")]
    fn initialize(&mut self, _client_info: Option<ClientInfo>, _locale: Option<String>) -> Result<InitializeResult, ResponseError> {
        Ok(InitializeResult{ 
            capabilities: ServerCapabilities{
                text_document_sync: TextDocumentSyncOptions{
                    open_close: true,
                    change: 2
                },
                hover_provider: true
            },
            server_info: ServerInfo{ 
                name: String::from("Descend LSP"), 
                version: String::from("1.0.0") 
            } 
        })
    }

    #[route("initialized")]
    fn initialized(&mut self) {
    }

    #[route("textDocument/didOpen")]
    fn did_open_text_document(&mut self, text_document: TextDocumentItem) {
        let text_documents_map= &mut self.state().text_documents;
        text_documents_map.insert(text_document.uri, TextDocument { 
            lines: text_document.text.split("\r\n").map(str::to_string).collect() 
        });
    }

    #[route("textDocument/didChange")]
    fn did_change_text_document(&mut self, text_document: TextDocumentIdentifier, content_changes: Vec<TextDocumentContentChangeEvent>) {
        let text_documents_map = &mut self.state().text_documents;
        for content_change in content_changes {
            let text_document = text_documents_map.get_mut(&text_document.uri).expect(&format!("Unknown document \"{}\"", text_document.uri));
            text_document.edit(&content_change.range, &content_change.text);
        }
    }

    #[route("textDocument/didClose")]
    fn did_close_text_document(&mut self, text_document: TextDocumentIdentifier) {
        let text_documents_map = &mut self.state().text_documents;
        text_documents_map.remove(&text_document.uri);
    }

    #[route("textDocument/hover")]
    fn hover(&mut self, text_document: TextDocumentIdentifier, position: Position) -> Result<Hover, ResponseError> {
        let text_documents_map = &mut self.state().text_documents;
        let text_document = text_documents_map.get_mut(&text_document.uri).expect(&format!("Unknown document \"{}\"", text_document.uri));
        Ok(Hover {
            contents: MarkupContent { 
                kind: String::from("plaintext"), 
                value: text_document.lines[position.line as usize][(position.character as usize)..].to_string()
            }
        })
    }
}

// Server state
pub struct State {
    pub stdin: std::io::Stdin,
    pub stdout: std::io::Stdout,
    pub text_documents: HashMap<String, TextDocument>
}

impl Router for State {
    fn state(&mut self) -> &mut State {
        self
    }
}

fn get_response(message: Result<RawMessage, String>, server: &mut impl Router) -> Option<ResponseMessage> {
    match message {
        Ok(message) => {
            let message = Message::from_raw(&message).unwrap();
            route_msg(server, message) // "route_msg" generated by the router macro
        },
        Err(error) => {
            Some(ResponseMessage::error(Id::AsJson(serde_json::Value::Null), ResponseError::INTERNAL_ERROR, error))
        }
    }
}

fn main() {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();

    let mut server = State{
        stdin,
        stdout,
        text_documents: HashMap::new()
    };

    loop {
        let message = RawMessage::read(&mut server.stdin.lock());
        let response = get_response(message, &mut server);

        if let Some(response) = response {
            let response = serde_json::to_value(response);
            if let Err(error) = response {
                eprintln!("{}", error.to_string());
                continue;
            }

            let response = RawMessage::from(response.unwrap());
            response.write(&mut server.stdout).unwrap_or(());
            server.stdout.flush().unwrap_or(());
        }
    }
}
