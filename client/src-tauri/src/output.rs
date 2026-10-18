//! Put the answer where the user wants it: pasted at the cursor, or on the clipboard.

use std::thread::sleep;
use std::time::Duration;

use anyhow::Result;

use crate::config::OutputMode;
use crate::platform::{self, PasteMethod};

pub enum Delivery {
    Pasted,
    /// On the clipboard; `reason` says why it wasn't pasted when pasting was wanted.
    Copied { reason: Option<String> },
}

enum Saved {
    Text(String),
    Image(arboard::ImageData<'static>),
}

impl Saved {
    fn capture(clipboard: &mut arboard::Clipboard) -> Option<Saved> {
        if let Ok(text) = clipboard.get_text() {
            return Some(Saved::Text(text));
        }
        clipboard.get_image().ok().map(|img| Saved::Image(img.to_owned_img()))
    }

    fn restore(self, clipboard: &mut arboard::Clipboard) {
        let _ = match self {
            Saved::Text(text) => clipboard.set_text(text),
            Saved::Image(img) => clipboard.set_image(img),
        };
    }
}

pub fn deliver(
    clipboard: &mut arboard::Clipboard,
    text: &str,
    output: OutputMode,
    paste_method: Option<PasteMethod>,
    restore_clipboard: bool,
) -> Result<Delivery> {
    let method = match output {
        OutputMode::Clipboard => None,
        OutputMode::Auto | OutputMode::Paste => paste_method,
    };
    let previous = if method.is_some() && restore_clipboard { Saved::capture(clipboard) } else { None };

    clipboard.set_text(text)?;

    let Some(method) = method else {
        let reason = (output == OutputMode::Paste).then(|| "pasting at the cursor isn't available on this system".to_string());
        return Ok(Delivery::Copied { reason });
    };

    // Give clipboard managers and the target app a moment to see the new contents.
    sleep(Duration::from_millis(80));
    if let Err(e) = platform::paste(method) {
        return Ok(Delivery::Copied { reason: Some(format!("couldn't paste ({e})")) });
    }

    if let Some(previous) = previous {
        // The target app reads the clipboard asynchronously after Ctrl+V.
        sleep(Duration::from_millis(600));
        // Only restore if nothing else has replaced the answer in the meantime.
        if clipboard.get_text().ok().as_deref() == Some(text) {
            previous.restore(clipboard);
        }
    }
    Ok(Delivery::Pasted)
}
