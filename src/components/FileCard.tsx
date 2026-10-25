import type { SelectedFile } from "../api";
import { formatSize } from "../format";
import { AlertIcon, CloseIcon, FileIcon, ImageIcon, TextIcon, MicIcon, VideoIcon } from "../icons";

interface Props {
  file: SelectedFile;
  /** Small preview for images, once it has been generated. */
  thumbnail: string | null;
  onChange: () => void;
  onRemove: () => void;
}

function KindIcon({ kind }: { kind: SelectedFile["kind"] }) {
  if (kind === "image") return <ImageIcon width={26} height={26} />;
  if (kind === "video") return <VideoIcon width={26} height={26} />;
  if (kind === "audio") return <MicIcon width={26} height={26} />;
  if (kind === "text" || kind === "pdf") return <TextIcon width={26} height={26} />;
  return <FileIcon width={26} height={26} />;
}

export function FileCard({ file, thumbnail, onChange, onRemove }: Props) {
  return (
    <section className="file-card" aria-label="Selected file">
      {thumbnail ? (
        <img className="file-thumb" src={thumbnail} alt="" />
      ) : (
        <div className={`file-badge kind-${file.kind}`}>
          <KindIcon kind={file.kind} />
        </div>
      )}
      <div className="file-meta">
        <h2 className="file-name" title={file.name}>
          {file.name}
        </h2>
        <p className="file-details">
          {file.typeLabel}
          <span aria-hidden="true"> · </span>
          {formatSize(file.sizeBytes)}
        </p>
        {file.extensionMismatch && (
          <p className="file-warning">
            <AlertIcon width={16} height={16} />
            Its name ends in “.{file.extension}”, but the contents are a {file.typeLabel}. MyFile goes by the contents.
          </p>
        )}
      </div>
      <div className="file-buttons">
        <button type="button" className="btn btn-secondary" onClick={onChange}>
          Change file
        </button>
        <button type="button" className="btn btn-icon" onClick={onRemove} aria-label="Remove file" title="Remove file">
          <CloseIcon />
        </button>
      </div>
    </section>
  );
}
