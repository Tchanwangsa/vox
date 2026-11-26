// Every word a visitor reads lives here, so the copy can change without touching layout.

export const REPO = "https://github.com/Tchanwangsa/vox";
export const RELEASES = `${REPO}/releases/latest`;
export const RELEASE_API = "https://api.github.com/repos/Tchanwangsa/vox/releases/latest";

export const hero = {
  sub: "Vox transcribes what you say, removes filler, fixes punctuation, and types into the app in front. Run it offline on your Mac or through Groq.",
  primary: "Download for macOS",
  secondary: "View on GitHub",
  note: "Free and open source. macOS 13+, Apple silicon.",
};

// The spoken sentence, split where Vox cuts: `cut` parts are the filler and
// the self-correction that never reach the page.
export const demo = {
  spoken: [
    { text: "Um,", cut: true },
    { text: "hey Sam, I sent over the new mockups," },
    { text: "uh,", cut: true },
    { text: "let's walk through them on" },
    { text: "Tuesday, sorry,", cut: true },
    { text: "Wednesday morning once you've had a look." },
  ],
  typed: "Hey Sam, I sent over the new mockups. Let's walk through them on Wednesday morning once you've had a look.",
};

export const steps = [
  {
    title: "Hold and speak",
    body: "Hold ⌃ ⌥ Space while you talk and release to finish, or tap it once for hands-free dictation.",
  },
  {
    title: "Vox tidies it up",
    body: "It transcribes the clip, drops fillers and false starts, and fixes punctuation.",
  },
  {
    title: "Typed where you are",
    body: "The finished text is typed straight into whichever app is in front, and saved to your history.",
  },
];

export const features = [
  {
    icon: "mic",
    title: "Hold or tap to talk",
    body: "Hold ⌃ ⌥ Space to speak, or tap once for hands-free dictation.",
  },
  {
    icon: "layers",
    title: "On this Mac or Groq",
    body: "Transcribe offline on your Mac, or use Groq with your own API key.",
  },
  {
    icon: "message",
    title: "Transcript cleanup",
    body: "Vox clears fillers, fixes punctuation, and types raw text if cleanup fails.",
  },
  {
    icon: "chart",
    title: "History and stats",
    body: "See recent dictations and word stats by day and app, with optional audio.",
  },
] as const;

export const screens = {
  heading: "Inside the app",
  hint: "Click the tabs in the window to look around.",
  tabs: [
    {
      id: "home",
      label: "Home",
      caption: "See your shortcut, switch engines, and browse recent dictations.",
    },
    {
      id: "stats",
      label: "Stats",
      caption: "Speaking time and word counts broken down by day and by app.",
    },
    {
      id: "engine",
      label: "Engine",
      caption: "Pick On this Mac or Groq, download local models, or save your API key.",
    },
    {
      id: "settings",
      label: "Settings",
      caption: "Pick a shortcut preset, record your own, or choose whether to save audio.",
    },
  ],
} as const;

// `mark` names a logo in public/brand; `tint` is the colour the app gives it.
// A row with more than one model is shown as a picker, each with its own note.
export const engines = {
  heading: "Where your voice goes",
  sub: "Keep audio offline on your Mac or send it to Groq, with optional fallback.",
  cards: [
    {
      name: "On this Mac",
      mark: undefined,
      tint: undefined,
      body: "Dictation runs offline, so nothing leaves your computer.",
      hears: [
        { mark: "openai", name: "OpenAI Whisper", note: "Runs with whisper.cpp on macOS 13+." },
        { mark: "cohere", name: "Cohere Transcribe", note: "Needs Apple silicon and macOS 14+ (MLX)." },
      ],
      tidies: [{ mark: "superwhisper", name: "S1-mini by Superwhisper" }],
    },
    {
      name: "Groq",
      mark: "groq",
      tint: "#f55036",
      body: "Audio is sent to Groq for any language, using your own API key.",
      hears: [{ mark: "openai", name: "Whisper large v3 turbo" }],
      tidies: [{ mark: "qwen", name: "Qwen 3.8 27B", tint: "#7563df" }],
    },
  ],
};

export const install = {
  heading: "Install on macOS",
  steps: [
    "Download Vox from **GitHub**, drag it into **Applications**, and open it.",
    "Vox is not notarized yet. If **macOS blocks it**, run this command in **Terminal** once.",
    "Allow **Microphone and Accessibility** in System Settings. Until Accessibility is on, dictations go to the **clipboard**.",
  ],
  unblock: "xattr -cr /Applications/Vox.app",
  settingsPath: "System Settings → Privacy & Security",
  localModelsNote: "Local models need whisper-cpp and llama.cpp installed with Homebrew.",
  brew: "brew install whisper-cpp llama.cpp",
};

export const openSource = {
  heading: "Free and open source",
  body: "Vox is MIT-licensed and written in Rust with GPUI. Models download on demand inside the app.",
  cta: "View on GitHub",
  // Building from source, as the README has it.
  commands: [`git clone ${REPO}`, "cd vox", "scripts/bundle.sh --open"],
  comment: "build target/Vox.app and launch it",
};

export const closing = {
  headline: "Click a text field and speak.",
  sub: "Free under the MIT license for macOS 13 and later on Apple silicon.",
};

export const footer = {
  line: "Vox is free, open-source macOS dictation written in Rust. MIT license.",
  author: { name: "Tanat Chanwangsa", url: "https://tchanwangsa.xyz" },
  marks: "S1-mini is a model by Superwhisper. Other names and marks belong to their owners, and Vox is not affiliated with them.",
};
