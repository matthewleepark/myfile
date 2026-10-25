// Turns demo/frames.txt (from record.mjs) into demo/myfile-demo.mp4.
// Needs ffmpeg on PATH or FFMPEG=path/to/ffmpeg.
import { execFileSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const ffmpeg = process.env.FFMPEG ?? "ffmpeg";
execFileSync(
  ffmpeg,
  [
    "-y",
    "-f", "concat",
    "-safe", "0",
    "-i", "frames.txt",
    // Constant 30 fps, even dimensions, and a pixel format every player supports.
    "-vf", "fps=30,scale=trunc(iw/2)*2:trunc(ih/2)*2,format=yuv420p",
    "-c:v", "libx264",
    "-preset", "slow",
    "-crf", "22",
    "-movflags", "+faststart",
    "myfile-demo.mp4",
  ],
  { cwd: here, stdio: "inherit" },
);
console.log("Wrote", join(here, "myfile-demo.mp4"));
