import { Code2, Download, File, FileText, Image, Table2 } from "lucide-react";

export type ChatArtifact = {
  artifact_id: string;
  name?: string;
  mime?: string;
  size?: number;
};

function formatSize(size?: number) {
  if (!size) return "";
  if (size < 1024) return `${size} B`;
  if (size < 1024 * 1024) return `${Math.round(size / 1024)} KB`;
  return `${(size / (1024 * 1024)).toFixed(1)} MB`;
}

function fileKind(mime = "", name = "") {
  const extension = name.split(".").pop()?.toUpperCase();
  if (mime.startsWith("image/")) return { label: "画像", Icon: Image };
  if (mime === "application/pdf") return { label: "PDF", Icon: FileText };
  if (["HTML", "CSS", "JS", "TS", "JSON", "MD"].includes(extension || "")) return { label: `コード・${extension}`, Icon: Code2 };
  if (["CSV", "XLS", "XLSX"].includes(extension || "")) return { label: "表計算", Icon: Table2 };
  return { label: extension ? `ファイル・${extension}` : "ファイル", Icon: File };
}

export function ArtifactCard({ artifact }: { artifact: ChatArtifact }) {
  const { label, Icon } = fileKind(artifact.mime, artifact.name);
  return (
    <section className="artifact-card">
      <div className="artifact-icon"><Icon aria-hidden="true" size={27} /></div>
      <div className="artifact-details">
        <b>{artifact.name || "成果物"}</b>
        <span>{label}{artifact.size ? ` · ${formatSize(artifact.size)}` : ""}</span>
      </div>
      <a className="artifact-download" href={`/api/v1/artifacts/${artifact.artifact_id}`} download={artifact.name || true}>
        <Download aria-hidden="true" size={17} /> ダウンロード
      </a>
    </section>
  );
}
