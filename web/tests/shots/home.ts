// The README screenshots' machine: a developer's /home/alex today and as the
// previous scan saw it yesterday, so Changes has a story to tell (a model
// pulled, an ISO downloaded, recordings deleted). Sizes and entry counts are
// made up but plausible; nothing here comes from a real disk.
import { DAY, G, M, MockApi, NOW, type Node } from "../e2e/mock";

type Day = "today" | "yesterday";

const K = 2 ** 10;
const dir = (name: string, mtime: number, items: number, children: Node[]): Node => ({
  name,
  kind: "dir",
  mtime: NOW - mtime,
  items,
  children,
});
const file = (name: string, allocated: number, mtime: number): Node => ({
  name,
  kind: "file",
  allocated: Math.round(allocated),
  mtime: NOW - mtime,
});
const h = 3600;

function home(day: Day): Node {
  const today = day === "today";
  const only = (n: Node, when: Day): Node[] => (day === when ? [n] : []);
  return dir("/", 30 * DAY, 0, [
    dir("home", 200 * DAY, 0, [
      dir("alex", 2 * h, 0, [
        dir(".cache", 3 * h, 184_312, [
          dir("huggingface", 3 * h, 1_204, [
            file("Llama-3.1-8B-Instruct.safetensors", 16 * G, 21 * DAY),
            ...only(file("Mistral-Small-24B.safetensors", 14.5 * G, 3 * h), "today"),
          ]),
          dir("pip", 300 * DAY, 41_877, [file("wheels", 6.1 * G, 300 * DAY)]),
          dir("go-build", 4 * DAY, 96_310, [file("objects", 3.2 * G, 4 * DAY)]),
          dir("JetBrains", 9 * DAY, 22_140, [file("indexes", 2.2 * G, 9 * DAY)]),
          dir("yarn", 33 * DAY, 18_502, [file("v6", 2.4 * G, 33 * DAY)]),
          dir("mozilla", 5 * h, 4_270, [file("cache2", 1.3 * G, 5 * h)]),
        ]),
        dir("Projects", 2 * h, 412_877, [
          dir("ml-notebooks", 6 * DAY, 61_032, [
            dir("data", 6 * DAY, 212, [file("events.parquet", 11 * G, 6 * DAY)]),
            dir(".venv", 40 * DAY, 58_400, [file("lib", 4.8 * G, 40 * DAY)]),
          ]),
          dir("rust-cli", 2 * h, 9_118, [dir("target", 2 * h, 8_300, [file("debug", (today ? 9.4 : 7.1) * G, 2 * h)])]),
          dir("webshop", 1 * DAY, 201_660, [
            dir("node_modules", 12 * DAY, 198_004, [file("packages", 2.1 * G, 12 * DAY)]),
            dir(".git", 1 * DAY, 3_120, [file("objects", 640 * M, 1 * DAY)]),
          ]),
          dir("old-blog", 610 * DAY, 141_065, [dir("node_modules", 610 * DAY, 140_870, [file("packages", 1.4 * G, 610 * DAY)])]),
        ]),
        dir("VirtualBox VMs", 120 * DAY, 9, [dir("Windows 11", 120 * DAY, 7, [file("Windows 11.vdi", 24 * G, 120 * DAY)])]),
        dir(".local", 1 * DAY, 96_442, [
          dir("share", 1 * DAY, 96_430, [
            dir("Steam", 15 * DAY, 71_220, [file("steamapps", 19 * G, 15 * DAY)]),
            dir("containers", 1 * DAY, 25_180, [file("storage", 3.4 * G, 1 * DAY)]),
          ]),
        ]),
        dir("Downloads", 4 * h, 312, [
          ...only(file("ubuntu-24.04.3-desktop-amd64.iso", 6.1 * G, 4 * h), "today"),
          file("DaVinci_Resolve_19.1_Linux.zip", 3.1 * G, 80 * DAY),
          file("debian-13.1.0-amd64-netinst.iso", 754 * M, 45 * DAY),
          file("podcasts", 1.9 * G, 62 * DAY),
        ]),
        dir("Videos", 1 * DAY, 1_206, [
          dir("2025-06 wedding", 480 * DAY, 1_180, [file("clips", 9.8 * G, 480 * DAY)]),
          ...only(dir("obs-recordings", 2 * DAY, 14, [file("2026-10-13 demo.mkv", 4.2 * G, 2 * DAY)]), "yesterday"),
        ]),
        ...only(dir(".ollama", 3 * h, 31, [dir("models", 3 * h, 28, [file("qwen3-14b", 8.9 * G, 3 * h)])]), "today"),
        dir("Pictures", 30 * DAY, 8_812, [file("Camera", 7.4 * G, 30 * DAY)]),
        dir(".rustup", 25 * DAY, 31_904, [file("toolchains", 3.1 * G, 25 * DAY)]),
        dir(".cargo", 2 * h, 48_660, [file("registry", today ? 2.6 * G : 2.42 * G, 2 * h)]),
        dir(".npm", 1 * DAY, 39_120, [file("_cacache", today ? 1.9 * G : 1.6 * G, 1 * DAY)]),
        dir("Documents", 3 * DAY, 2_418, [file("scans", today ? 1.2 * G : 1.19 * G, 3 * DAY)]),
        dir(".config", 2 * h, 6_031, [file("Code", 640 * M, 2 * h)]),
        dir(".vscode", 7 * DAY, 3_877, [file("extensions", 480 * M, 7 * DAY)]),
        file(".bash_history", (today ? 1204 : 1196) * K, 2 * h),
      ]),
    ]),
  ]);
}

/** The mock API on that machine: a 238 G disk with 14 G left. */
export class ShotApi extends MockApi {
  constructor() {
    super(false);
    this.host = "workstation";
    this.tree = home("today");
    this.prevTree = home("yesterday");
  }

  override status() {
    const s = super.status();
    return { ...s, roots: s.roots.map((r) => ({ ...r, fs_used: 224 * G, fs_total: 238 * G, fs_avail: 14 * G })) };
  }
}
