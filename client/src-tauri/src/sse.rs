//! Minimal server-sent events parser: feed it chunks, get back complete events.

#[derive(Debug, PartialEq, Eq)]
pub struct Event {
    pub event: String,
    pub data: String,
}

#[derive(Default)]
pub struct Parser {
    buffer: String,
}

impl Parser {
    pub fn feed(&mut self, chunk: &str) -> Vec<Event> {
        self.buffer.push_str(&chunk.replace("\r\n", "\n"));
        let mut events = Vec::new();
        while let Some(end) = self.buffer.find("\n\n") {
            let frame: String = self.buffer.drain(..end + 2).collect();
            if let Some(event) = parse_frame(&frame) {
                events.push(event);
            }
        }
        events
    }
}

fn parse_frame(frame: &str) -> Option<Event> {
    let mut event = String::from("message");
    let mut data: Vec<&str> = Vec::new();
    for line in frame.lines() {
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => event = value.to_string(),
            "data" => data.push(value),
            _ => {}
        }
    }
    (!data.is_empty()).then(|| Event { event, data: data.join("\n") })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_events_split_across_chunks() {
        let mut parser = Parser::default();
        assert!(parser.feed("event: delta\ndata: {\"text\":").is_empty());
        let events = parser.feed("\"Hi\"}\n\n: keepalive\n\nevent: done\r\ndata: {}\r\n\r\n");
        assert_eq!(
            events,
            vec![
                Event { event: "delta".into(), data: "{\"text\":\"Hi\"}".into() },
                Event { event: "done".into(), data: "{}".into() },
            ]
        );
    }

    #[test]
    fn comment_only_frames_are_skipped() {
        assert!(Parser::default().feed(": keepalive\n\n").is_empty());
    }
}
