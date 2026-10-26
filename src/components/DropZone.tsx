import { UploadIcon } from "../icons";

interface Props {
  dragging: boolean;
  onBrowse: () => void;
  onBrowseFolder: () => void;
}

/** The empty-state target. The actual drop handling is window-wide (see useFileDrop). */
export function DropZone({ dragging, onBrowse, onBrowseFolder }: Props) {
  return (
    <section className={`dropzone${dragging ? " is-dragging" : ""}`} aria-label="Choose a file">
      <div className="dropzone-icon">
        <UploadIcon width={28} height={28} />
      </div>
      <h2 className="dropzone-title">{dragging ? "Release to convert this file" : "Drop any file to convert it"}</h2>
      <p className="dropzone-sub">MyFile shows what it can become. Drop a folder to pull everything out of it into one file.</p>
      <div className="dropzone-buttons">
        <button type="button" className="btn btn-primary" onClick={onBrowse}>
          Choose a file
        </button>
        <button type="button" className="btn btn-secondary" onClick={onBrowseFolder}>
          Choose a folder
        </button>
      </div>
      <p className="dropzone-formats">
        Images · PDFs · Word, Excel and PowerPoint · Markdown and text · CSV, JSON, YAML · audio · video · subtitles · ZIP
      </p>
    </section>
  );
}
