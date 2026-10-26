// Typed wrappers around MyFile's Rust commands. These types mirror the
// structs in src-tauri/src; keep them in sync when either side changes.

import { invoke } from "@tauri-apps/api/core";

export type FileKind = "image" | "pdf" | "text" | "audio" | "video" | "unsupported";

export type ActionId =
  | "preview-text"
  | "extract-pdf-text"
  | "read-image-text"
  | "compare-document"
  | "monitor-document"
  | "extract-fields"
  | "transcribe"
  | "compress-image"
  | "ai-assistant";

export interface ActionInfo {
  id: ActionId;
  label: string;
  description: string;
  usesAi: boolean;
}

export interface SelectedFile {
  /** Names this selection; AI calls must pass it so they can't act on a different file. */
  selectionId: number;
  name: string;
  extension: string;
  sizeBytes: number;
  kind: FileKind;
  typeLabel: string;
  extensionMismatch: boolean;
  actions: ActionInfo[];
}

export interface TextPreview {
  text: string;
  truncated: boolean;
}

export interface ExtractedText {
  text: string;
  truncatedForDisplay: boolean;
  pageCount: number;
  emptyPages: number;
  charCount: number;
  outputId: number;
  /** Read with OCR (a scan or a photo), so it may contain misreads. */
  ocr: boolean;
}

export interface CompressOptions {
  format: "jpeg" | "png";
  quality: number;
  maxDimension: number | null;
}

export interface CompressResult {
  outputId: number;
  fileName: string;
  originalBytes: number;
  outputBytes: number;
  originalWidth: number;
  originalHeight: number;
  width: number;
  height: number;
  previewDataUrl: string;
}

export type AiTask = "summarize" | "deadlines" | "requirements" | "decisions" | "simplify" | "question";

export interface AiStatus {
  available: boolean;
  provider: string;
  model: string | null;
  maxChars: number;
}

export interface AiItem {
  title: string;
  detail: string;
  category: string | null;
  date: string | null;
  /** YYYY-MM-DD, only when MyFile's code confirmed it matches the written date. */
  isoDate: string | null;
  quote: string | null;
  quoteCheck: "verified" | "not-found" | null;
  /** Whether the date appears in the item's own quote; "unsupported" dates are kept off calendars. */
  dateCheck: "supported" | "unsupported" | null;
  /** The quote comes from text addressed to an AI (likely prompt injection). */
  suspicious: boolean;
  page: number | null;
  basis: "explicit" | "inferred";
}

export interface AiResult {
  /** For recordings: when each item's quote was said (ms), by item index. */
  itemTimes?: (number | null)[] | null;
  task: AiTask;
  answer: string;
  found: boolean;
  items: AiItem[];
  notes: string[];
  model: string;
  durationMs: number;
  charsSent: number;
  usage: AiUsage;
  checks: AiChecks;
  outputId: number;
  /** Present for deadline answers with at least one checked calendar date. */
  calendar: { outputId: number; events: number; skipped: number } | null;
}

export interface AiUsage {
  /** The CLI's list-price estimate; on a subscription it counts against plan limits instead. */
  costUsd: number;
  inputTokens: number;
  outputTokens: number;
  apiMs: number;
}

export interface AiChecks {
  quotesChecked: number;
  quotesVerified: number;
  pagesCorrected: number;
  pagesRemoved: number;
  datesUnsupported: number;
  /** Phrases in the document that look like instructions to an AI. */
  injectionPhrases: string[];
  itemsFromInjectedText: number;
}

export interface AppError {
  code: string;
  message: string;
}

/** Rust commands reject with an AppError; anything else is unexpected. */
export function toAppError(e: unknown): AppError {
  if (e && typeof e === "object" && "code" in e && "message" in e) {
    return e as AppError;
  }
  return { code: "unexpected", message: `Something unexpected went wrong: ${String(e)}` };
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw toAppError(e);
  }
}

export const api = {
  selectFile: (path: string) => call<SelectedFile>("select_file", { path }),
  clearFile: () => call<void>("clear_file"),
  shiftHeld: () => call<boolean>("shift_held"),
  previewText: () => call<TextPreview>("preview_text"),
  extractPdfText: () => call<ExtractedText>("extract_pdf_text"),
  imageThumbnail: () => call<string>("image_thumbnail"),
  compressImage: (options: CompressOptions) => call<CompressResult>("compress_image", { options }),
  aiStatus: () => call<AiStatus>("ai_status"),
  grantAiConsent: (selectionId: number) => call<void>("grant_ai_consent", { selectionId }),
  runAi: (selectionId: number, task: AiTask, question: string | null) =>
    call<AiResult>("run_ai", { selectionId, task, question }),
  cancelAi: () => call<void>("cancel_ai"),
  /** Same task on a local Ollama model; the text never leaves this computer. */
  runAiLocal: (selectionId: number, task: AiTask, question: string | null) =>
    call<AiResult>("run_ai_local", { selectionId, task, question }),
  localStatus: () => call<{ available: boolean; model: string; message: string | null }>("local_status"),
  cancelLocal: () => call<void>("cancel_local"),
  outputText: (outputId: number) => call<string>("output_text", { outputId }),
  /** Resolves to the saved filename, or null if the user cancelled. */
  saveOutput: (outputId: number) => call<string | null>("save_output", { outputId }),
};

export interface DocumentStats {
  chars: number;
  pages: number | null;
  ocr: boolean;
  /** The text is a machine transcript of a recording. */
  transcript: boolean;
}

export const documentStats = (selectionId: number) => call<DocumentStats>("document_stats", { selectionId });

export interface RouteDecision {
  action: ActionId | "none";
  aiTask: AiTask | null;
  question: string | null;
  imageOptions: CompressOptions | null;
  reason: string;
  /** Settings MyFile's own code changed because the model's were out of range. */
  adjusted: string[];
  model: string;
  usage: AiUsage;
}

/** Sends only the typed request and the file's type (never its contents) to pick an action. */
export const routeRequest = (selectionId: number, request: string) =>
  call<RouteDecision>("route_request", { selectionId, request });

// ---- Semester brain (folder mode) ----

export interface FolderDocSummary {
  index: number;
  name: string;
  kind: FileKind;
  pages: number | null;
  chars: number;
  error: string | null;
}

export interface FolderSummary {
  selectionId: number;
  name: string;
  docs: FolderDocSummary[];
  images: { index: number; name: string; sizeBytes: number }[];
  totalChars: number;
  skipped: string[];
  /** Every file in the folder and its subfolders. */
  fileCount: number;
  totalBytes: number;
}

export interface FolderProgress {
  phase: "reading" | "analyzing";
  done: number;
  total: number;
  current: string | null;
}

export interface TimelineEntry {
  docIndex: number;
  docName: string;
  title: string;
  detail: string;
  category: string | null;
  date: string | null;
  isoDate: string | null;
  quote: string | null;
  quoteCheck: "verified" | "not-found" | null;
  dateCheck: "supported" | "unsupported" | null;
  /** The quote comes from text addressed to an AI (likely prompt injection). */
  suspicious: boolean;
  page: number | null;
  basis: "explicit" | "inferred";
}

export interface FolderAnalysis {
  timeline: TimelineEntry[];
  conflicts: { isoDate: string; entries: number[]; docs: string[] }[];
  crunchWeeks: { weekStart: string; count: number; docs: string[] }[];
  perDoc: { index: number; name: string; items: number; quotesChecked: number; quotesVerified: number; error: string | null }[];
  calendar: { outputId: number; events: number } | null;
  model: string;
  usage: AiUsage;
  durationMs: number;
}

export interface AskItem {
  source: string | null;
  sourceCorrected: boolean;
  title: string;
  detail: string;
  date: string | null;
  quote: string | null;
  quoteCheck: "verified" | "not-found" | null;
  page: number | null;
}

export interface AskResult {
  answer: string;
  found: boolean;
  items: AskItem[];
  notes: string[];
  model: string;
  usage: AiUsage;
  durationMs: number;
}

export const folderApi = {
  open: (path: string) => call<FolderSummary>("open_folder", { path }),
  close: () => call<void>("close_folder"),
  grantConsent: (selectionId: number) => call<void>("grant_folder_consent", { selectionId }),
  cancel: () => call<void>("cancel_folder"),
  /** providerChoice: "local" (default where installed, nothing leaves the computer) or "claude". */
  analyze: (selectionId: number, providerChoice: "local" | "claude") =>
    call<FolderAnalysis>("analyze_folder", { selectionId, providerChoice }),
  ask: (selectionId: number, question: string, providerChoice: "local" | "claude") =>
    call<AskResult>("ask_folder", { selectionId, question, providerChoice }),
};

// ---- Agent mode ----

export type AgentTool = "find_deadlines" | "summarize" | "ask" | "find_conflicts" | "build_calendar" | "compress_image";

export interface AgentStep {
  index: number;
  tool: AgentTool;
  target: string | null;
  reason: string;
  usesAi: boolean;
}

export interface AgentPlan {
  planId: number;
  summary: string;
  steps: AgentStep[];
  docsSent: string[];
  adjusted: string[];
  model: string;
  usage: AiUsage;
}

export interface AgentStepResult {
  index: number;
  status: "running" | "done" | "error";
  message: string;
  outputId: number | null;
  outputLabel: string | null;
}

export interface AgentRun {
  steps: AgentStepResult[];
  briefing: string | null;
  timelineItems: number;
  conflicts: number;
  usage: AiUsage;
  durationMs: number;
}

export const agentApi = {
  /** Only the folder's inventory (names, types, sizes) and the goal are sent. */
  plan: (selectionId: number, goal: string) => call<AgentPlan>("plan_agent", { selectionId, goal }),
  /** Runs the stored, validated plan; the UI can only approve it by id. */
  run: (planId: number) => call<AgentRun>("run_agent", { planId }),
  discard: () => call<void>("discard_plan"),
};

// ---- Folder watch ----

export interface WatchInfo {
  watchId: number;
  folder: string;
  autoRead: boolean;
}

export interface WatchEvent {
  watchId: number;
  seq: number;
  name: string;
  path: string;
  status: "new" | "reading" | "done" | "error";
  message: string;
  items: number;
  calendarEvents: number;
}

export const watchApi = {
  start: (path: string, autoRead: boolean) => call<WatchInfo>("start_watch", { path, autoRead }),
  stop: () => call<void>("stop_watch"),
  saveCalendar: (watchId: number, seq: number) => call<string | null>("save_watch_calendar", { watchId, seq }),
};

// ---- Document comparison ----

export interface Change {
  id: number;
  kind: "added" | "removed" | "changed";
  before: string | null;
  after: string | null;
  beforePage: number | null;
  afterPage: number | null;
  words: { kind: "same" | "removed" | "added"; text: string }[];
  /** Found by code: e.g. "Nov 12, 2026 → Nov 17, 2026". */
  dateChange: string | null;
}

export interface Comparison {
  comparisonId: number;
  oldName: string;
  newName: string;
  currentIsNew: boolean;
  diff: { changes: Change[]; hiddenChanges: number; added: number; removed: number; changed: number; unchanged: number };
  explainChars: number;
  ocr: boolean;
}

export interface Explanation {
  summary: string;
  points: { title: string; detail: string; changes: number[]; importance: "high" | "medium" | "low" }[];
  notes: string[];
  unmentionedDateChanges: number[];
  /** Changes no point mentions, found by code. */
  unexplainedChanges: number[];
  model: string;
  usage: AiUsage;
  durationMs: number;
}

export const compareApi = {
  compare: (selectionId: number, otherPath: string, currentIsNew: boolean | null) =>
    call<Comparison>("compare_with", { selectionId, otherPath, currentIsNew }),
  grantConsent: (comparisonId: number) => call<void>("grant_comparison_consent", { comparisonId }),
  explain: (comparisonId: number, providerChoice: "local" | "claude") =>
    call<Explanation>("explain_comparison", { comparisonId, providerChoice }),
};

// ---- Change monitor ----

export interface MonitorItem {
  id: number;
  name: string;
  status: "unchanged" | "metadata-only" | "text-changed" | "missing" | "unreadable";
  /** Computed by code, e.g. "Date changed: Nov 12, 2026 → Nov 17, 2026 · 1 added". */
  summary: string;
  snapshotTakenAt: number;
}

export const monitorApi = {
  add: (selectionId: number) => call<MonitorItem>("monitor_add", { selectionId }),
  list: () => call<MonitorItem[]>("monitor_list"),
  changes: (id: number) => call<Comparison>("monitor_changes", { id }),
  accept: (id: number) => call<void>("monitor_accept", { id }),
  remove: (id: number) => call<void>("monitor_remove", { id }),
};

// ---- Fact checking ----

export interface FactCheck {
  claim: string;
  /** What the model said. */
  modelVerdict: "supported" | "contradicted" | "ambiguous" | "not-found" | "unverified";
  /** What MyFile shows after checking the evidence (may be downgraded). */
  verdict: "supported" | "contradicted" | "ambiguous" | "not-found" | "unverified";
  explanation: string;
  missing: string | null;
  evidence: {
    quote: string;
    stance: "supports" | "contradicts" | "context";
    source: string | null;
    page: number | null;
    quoteCheck: "verified" | "not-found";
  }[];
  notes: string[];
  model: string;
  usage: AiUsage;
  durationMs: number;
}

export const factCheckApi = {
  file: (selectionId: number, claim: string, providerChoice: "local" | "claude") =>
    call<FactCheck>("fact_check_file", { selectionId, claim, providerChoice }),
  folder: (selectionId: number, claim: string, providerChoice: "local" | "claude") =>
    call<FactCheck>("fact_check_folder", { selectionId, claim, providerChoice }),
};

// ---- Extraction templates ----

export type FieldType = "text" | "number" | "money" | "date" | "boolean";

export interface Template {
  id: string;
  name: string;
  fields: { key: string; label: string; type: FieldType }[];
  builtIn: boolean;
}

export interface TemplateCell {
  key: string;
  value: string | null;
  normalized: string | null;
  quote: string | null;
  page: number | null;
  status: "verified" | "unverified" | "wrong-type" | "not-found";
  problem: string | null;
}

export interface TemplateTable {
  template: Template;
  rows: { document: string; cells: TemplateCell[]; error: string | null }[];
  csvOutputId: number;
  jsonOutputId: number;
  model: string;
  usage: AiUsage;
  durationMs: number;
}

export const extractApi = {
  list: () => call<Template[]>("list_templates"),
  save: (template: { name: string; fields: { label: string; type: FieldType }[] }) => call<Template>("save_template", { template }),
  remove: (id: string) => call<void>("delete_template", { id }),
  file: (selectionId: number, templateId: string, providerChoice: "local" | "claude") =>
    call<TemplateTable>("extract_file", { selectionId, templateId, providerChoice }),
  folder: (selectionId: number, templateId: string, providerChoice: "local" | "claude") =>
    call<TemplateTable>("extract_folder", { selectionId, templateId, providerChoice }),
};

// ---- Duplicates ----

export interface DuplicateGroup {
  kind: "identical" | "same-text" | "versions" | "similar-images";
  files: { name: string; sizeBytes: number; modified: number | null; newest: boolean }[];
  similarity: number | null;
  detail: string;
  dateChanges: string[];
}

export interface DuplicateReport {
  groups: DuplicateGroup[];
  filesChecked: number;
  reclaimableBytes: number;
}

export const duplicatesApi = {
  find: (selectionId: number) => call<DuplicateReport>("find_duplicates", { selectionId }),
};

// ---- Speech to text ----

export interface SpeechStatus {
  modelInstalled: boolean;
  modelLabel: string;
  downloadBytes: number;
  ffmpeg: boolean;
}

export interface Transcript {
  segments: { startMs: number; endMs: number; text: string }[];
  durationMs: number;
  elapsedMs: number;
  words: number;
  model: string;
  decoder: "builtin" | "ffmpeg";
  textOutputId: number;
  srtOutputId: number;
  vttOutputId: number;
  actions: ActionInfo[];
}

export const speechApi = {
  status: () => call<SpeechStatus>("speech_status"),
  download: () => call<void>("download_speech_model"),
  transcribe: (selectionId: number) => call<Transcript>("transcribe", { selectionId }),
  current: (selectionId: number) => call<Transcript | null>("current_transcript", { selectionId }),
  cancel: () => call<void>("cancel_speech"),
};

// ---- Search your files ----

export interface IndexStatus {
  roots: { path: string; name: string; files: number; failed: number; passages: number; indexedAt: number | null }[];
  passages: number;
  vectors: number;
  ollama: boolean;
  embedModelInstalled: boolean;
  embedModel: string;
  running: boolean;
}

export interface IndexReport {
  added: number;
  updated: number;
  removed: number;
  unchanged: number;
  failed: number;
  skippedOverLimit: number;
  embedded: number;
}

export interface SearchHit {
  path: string;
  name: string;
  folder: string;
  kind: string;
  passages: { chunkId: number; page: number | null; snippet: { text: string; hit: boolean }[]; matchedBy: "words" | "meaning" | "both" }[];
}

export interface SearchResult {
  hits: SearchHit[];
  mode: string;
  elapsedMs: number;
}

export interface PreparedAsk {
  askId: number;
  sources: { name: string; path: string; pages: number[]; passages: number }[];
  chars: number;
}

export interface FilesAnswer {
  answer: string;
  found: boolean;
  items: AskItem[];
  notes: string[];
  paths: Record<string, string>;
  model: string;
  usage: AiUsage;
  durationMs: number;
}

export const searchApi = {
  status: () => call<IndexStatus>("search_status"),
  addFolder: (path: string) => call<IndexReport>("add_search_folder", { path }),
  removeFolder: (path: string) => call<void>("remove_search_folder", { path }),
  reindex: () => call<IndexReport>("reindex"),
  stop: () => call<void>("stop_indexing"),
  deleteIndex: () => call<void>("delete_search_index"),
  enableMeaning: () => call<IndexReport>("enable_meaning_search"),
  search: (query: string) => call<SearchResult>("search_files", { query }),
  prepareAsk: (question: string) => call<PreparedAsk>("prepare_ask", { question }),
  ask: (askId: number, providerChoice: "local" | "claude") => call<FilesAnswer>("ask_files", { askId, providerChoice }),
};

// ---- Show in document ----

export interface Evidence {
  name: string;
  page: number | null;
  pageCount: number;
  before: string;
  matched: string;
  after: string;
  found: boolean;
}

export const evidenceApi = {
  current: (selectionId: number, quote: string | null, page: number | null, stay: boolean) =>
    call<Evidence>("evidence_current", { selectionId, quote, page, stay }),
  folder: (selectionId: number, name: string, quote: string | null, page: number | null, stay: boolean) =>
    call<Evidence>("evidence_folder", { selectionId, name, quote, page, stay }),
  indexed: (path: string, quote: string | null, page: number | null, stay: boolean) =>
    call<Evidence>("evidence_indexed", { path, quote, page, stay }),
  passage: (chunkId: number, query: string | null) => call<[string, Evidence]>("evidence_passage", { chunkId, query }),
};

// ---- Conversion ----

export interface ConvertTarget {
  id: string;
  label: string;
  hint: string | null;
  group: string;
  engine: "builtin" | "ffmpeg" | "libreoffice" | "pandoc";
  available: boolean;
  knobs: ("quality" | "size" | "bitrate")[];
}

export interface ConversionChoices {
  format: string | null;
  formatLabel: string | null;
  targets: ConvertTarget[];
  missingTools: string[];
}

export interface ConvertSettings {
  quality: number | null;
  maxDimension: number | null;
  bitrate: number | null;
}

export interface ConversionResult {
  conversionId: number;
  files: { name: string; sizeBytes: number }[];
  totalBytes: number;
  originalBytes: number;
  preview: string | null;
  elapsedMs: number;
  engine: ConvertTarget["engine"];
}

export interface Suggestion {
  target: string | null;
  quality: number | null;
  maxDimension: number | null;
  bitrate: number | null;
  reason: string;
  adjusted: string[];
  model: string;
}

// ---- Folder report: everything in a folder, as one file ----

export interface ReportFile {
  path: string;
  label: string;
  sizeBytes: number;
  details: string[];
  words: number;
  note: string | null;
  recording: boolean;
  transcribed: boolean;
}

export interface ReportSummary {
  files: ReportFile[];
  totalBytes: number;
  totalWords: number;
  read: number;
  notes: string[];
  elapsedMs: number;
}

export interface ReportProgress {
  done: number;
  total: number;
  current: string | null;
}

export const reportApi = {
  read: (selectionId: number, transcribe: boolean) => call<ReportSummary>("read_folder_report", { selectionId, transcribe }),
  /** target: docx, pdf, md, html, txt, json, or csv (the file list). Save it with convertApi.save. */
  export: (selectionId: number, target: string) => call<ConversionResult>("export_folder_report", { selectionId, target }),
};

export const convertApi = {
  choices: (selectionId: number) => call<ConversionChoices>("conversion_choices", { selectionId }),
  convert: (selectionId: number, target: string, settings: ConvertSettings) =>
    call<ConversionResult>("convert_file", { selectionId, target, settings }),
  cancel: () => call<void>("cancel_convert"),
  save: (conversionId: number) => call<{ name: string; count: number } | null>("save_conversion", { conversionId }),
  suggest: (selectionId: number, request: string) => call<Suggestion>("suggest_conversion", { selectionId, request }),
};
