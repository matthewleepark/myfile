//! The allowlist of operations MyFile can perform.
//!
//! Only actions that are actually implemented belong here. Each one maps to a
//! fixed Rust command; nothing (including the AI model) can ask for anything
//! outside this list.

use serde::Serialize;

use crate::files::FileKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ActionId {
    PreviewText,
    ExtractPdfText,
    ReadImageText,
    CompareDocument,
    MonitorDocument,
    ExtractFields,
    CompressImage,
    AiAssistant,
    Transcribe,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionInfo {
    pub id: ActionId,
    pub label: &'static str,
    /// One sentence saying what will happen if the user picks this action.
    pub description: &'static str,
    /// True if the action sends content to an AI model.
    pub uses_ai: bool,
}

const PREVIEW_TEXT: ActionInfo = ActionInfo {
    id: ActionId::PreviewText,
    label: "Preview text",
    description: "Show the beginning of the file here. Nothing is changed or sent anywhere.",
    uses_ai: false,
};

const EXTRACT_PDF_TEXT: ActionInfo = ActionInfo {
    id: ActionId::ExtractPdfText,
    label: "Extract PDF text",
    description: "Pull out the readable text on this computer so you can copy it or save it as a .txt file.",
    uses_ai: false,
};

const COMPRESS_IMAGE: ActionInfo = ActionInfo {
    id: ActionId::CompressImage,
    label: "Compress or resize image",
    description: "Make a smaller copy as JPEG or PNG, optionally scaled down. Your original isn't changed.",
    uses_ai: false,
};

const READ_IMAGE_TEXT: ActionInfo = ActionInfo {
    id: ActionId::ReadImageText,
    label: "Read text in image (OCR)",
    description: "Recognize the words in this photo or screenshot on this computer, so you can copy or save them.",
    uses_ai: false,
};

const COMPARE_DOCUMENT: ActionInfo = ActionInfo {
    id: ActionId::CompareDocument,
    label: "Compare with another version",
    description: "See exactly what changed between two versions, with every change linked to both texts.",
    uses_ai: false,
};

const MONITOR_DOCUMENT: ActionInfo = ActionInfo {
    id: ActionId::MonitorDocument,
    label: "Monitor for changes",
    description: "Keep a snapshot of this document's text and get told exactly what changes, like a moved deadline.",
    uses_ai: false,
};

const EXTRACT_FIELDS: ActionInfo = ActionInfo {
    id: ActionId::ExtractFields,
    label: "Fill a template",
    description: "Pull specific fields (like rent, deadline, or salary) into a checked table you can export.",
    uses_ai: true,
};

const TRANSCRIBE: ActionInfo = ActionInfo {
    id: ActionId::Transcribe,
    label: "Transcribe to text",
    description: "Turn the speech in this recording into text with timestamps, on this computer. Then summarize it, find deadlines, or save subtitles.",
    uses_ai: true,
};

const AI_ASSISTANT: ActionInfo = ActionInfo {
    id: ActionId::AiAssistant,
    label: "Ask the AI assistant",
    description: "Summarize, find deadlines and requirements, or ask your own question about this document.",
    uses_ai: true,
};

/// Actions for a file type. Reading text in images (and asking the AI about
/// them) is only offered when OCR is installed, so no button promises
/// something that can't run.
pub fn for_kind(kind: FileKind, ocr: bool) -> Vec<ActionInfo> {
    match kind {
        FileKind::Text => vec![AI_ASSISTANT, EXTRACT_FIELDS, PREVIEW_TEXT, COMPARE_DOCUMENT, MONITOR_DOCUMENT],
        FileKind::Pdf => vec![AI_ASSISTANT, EXTRACT_FIELDS, EXTRACT_PDF_TEXT, COMPARE_DOCUMENT, MONITOR_DOCUMENT],
        FileKind::Image if ocr => vec![COMPRESS_IMAGE, READ_IMAGE_TEXT, AI_ASSISTANT, EXTRACT_FIELDS],
        FileKind::Image => vec![COMPRESS_IMAGE],
        // The other actions appear once there's a transcript (see after_transcript).
        FileKind::Audio | FileKind::Video => vec![TRANSCRIBE],
        FileKind::Unsupported => vec![],
    }
}

/// What a recording offers once it has been transcribed.
pub fn after_transcript() -> Vec<ActionInfo> {
    vec![TRANSCRIBE, AI_ASSISTANT, EXTRACT_FIELDS]
}
