import { createMemo, createSignal, For, onSettled } from "solid-js";
import { demo, steps } from "./copy";
import { icons } from "./Icons";

// One dictation, start to finish, on a loop. Times are milliseconds into the cycle.
const TICK = 90;
const LISTEN_AT = 1300;
const HEAR_AT = 8500;
const DONE_AT = 10000;
const FADE_AT = 15000;
const CYCLE = 15800;

// When each of the three steps begins and ends, for the bars under the stage.
const SPANS = [
  [0, HEAR_AT],
  [HEAR_AT, DONE_AT],
  [DONE_AT, FADE_AT],
];

// The pill's waveform in the app: thirty bars, the newest on the right.
const BARS = 30;
const QUIET = Array.from({ length: BARS }, () => 0.02);
const STILL = QUIET.map((_, i) => 0.12 + 0.62 * Math.abs(Math.sin(i * 0.8)) * (0.55 + 0.45 * Math.sin(i * 0.31)));

const WORDS = demo.spoken.flatMap((part) =>
  part.text.split(" ").map((word) => ({ word, cut: "cut" in part && part.cut === true })),
);
const TYPED_WORDS = demo.typed.split(/\s+/).length;
// Leave a beat before the first word and after the last.
const WORD_EVERY = (HEAR_AT - LISTEN_AT - 900) / WORDS.length;

type Phase = "idle" | "listening" | "hearing" | "done";

export default function Demo() {
  let stage!: HTMLDivElement;
  // With motion turned down the demo holds still, one step at a time.
  const still = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  const [at, setAt] = createSignal(still ? DONE_AT : 0);
  const [levels, setLevels] = createSignal(still ? STILL : QUIET);

  const phase = createMemo<Phase>(() => {
    const t = at();
    if (t >= FADE_AT || t < LISTEN_AT) return "idle";
    if (t < HEAR_AT) return "listening";
    return t < DONE_AT ? "hearing" : "done";
  });
  const spoken = createMemo(() => {
    const t = at();
    if (t >= FADE_AT || t < LISTEN_AT) return 0;
    return Math.max(0, Math.min(WORDS.length, Math.floor((t - LISTEN_AT - 350) / WORD_EVERY) + 1));
  });
  const seconds = createMemo(() => Math.floor(Math.max(0, Math.min(at(), HEAR_AT) - LISTEN_AT) / 1000));
  const step = createMemo(() => (phase() === "done" ? 2 : phase() === "hearing" ? 1 : 0));
  const progress = (i: number) => {
    if (still) return step() >= i ? 1 : 0;
    const [from, to] = SPANS[i];
    return Math.max(0, Math.min(1, (at() - from) / (to - from)));
  };

  // Clicking a step plays the demo from there.
  const jump = (i: number) => {
    if (still) {
      setAt([HEAR_AT - 1, DONE_AT - 1, DONE_AT][i]);
      return;
    }
    if (i === 0) setLevels(QUIET);
    setAt([LISTEN_AT - TICK, HEAR_AT, DONE_AT][i]);
  };

  onSettled(() => {
    if (still) return;

    let timer: number | undefined;
    let beat = 0;
    const tick = () => {
      const next = (at() + TICK) % CYCLE;
      setAt(next);
      if (next < TICK) setLevels(QUIET);
      if (next >= LISTEN_AT && next < HEAR_AT) {
        beat += 1;
        // Syllables inside the slower rise and fall of a phrase.
        const syllable = Math.abs(Math.sin(beat * 0.55)) ** 1.5;
        const phrase = 0.55 + 0.45 * Math.sin(beat * 0.13 + 1);
        const level = Math.min(1, 0.05 + syllable * phrase * (0.5 + Math.random() * 0.65));
        setLevels((old) => [...old.slice(1), level]);
      }
    };
    const run = (on: boolean) => {
      if (on && timer === undefined) timer = window.setInterval(tick, TICK);
      if (!on && timer !== undefined) {
        window.clearInterval(timer);
        timer = undefined;
      }
    };
    // Only play while the stage is on screen and the tab is in front.
    let visible = false;
    const sync = () => run(visible && !document.hidden);
    const watcher = new IntersectionObserver(
      ([entry]) => {
        visible = entry.isIntersecting;
        sync();
      },
      { threshold: 0.25 },
    );
    watcher.observe(stage);
    document.addEventListener("visibilitychange", sync);
    return () => {
      run(false);
      watcher.disconnect();
      document.removeEventListener("visibilitychange", sync);
    };
  });

  return (
    <div class="demo">
      <p class="sr-only">
        An animated example. Someone says: {demo.spoken.map((part) => part.text).join(" ")} Vox types: {demo.typed}
      </p>
      <div class="stage" ref={(el) => (stage = el)} aria-hidden="true">
        <div class="window">
          <div class="window-bar">
            <i />
            <i />
            <i />
            <span>New message</span>
          </div>
          <div class="window-to">
            To <b>Sam</b>
          </div>
          <div class={{ "window-body": true, on: phase() === "done" }}>
            <span class="text">{demo.typed}</span>
            <span class="caret" />
          </div>
        </div>

        {/* What is being said, like captions above the pill. */}
        <p class={{ said: true, tidied: phase() === "hearing" || phase() === "done", gone: phase() === "done" }}>
          {WORDS.map((item, i) => (
            <span class={{ word: true, cut: item.cut, on: i < spoken() || phase() === "hearing" || phase() === "done" }}>
              {item.word}{" "}
            </span>
          ))}
        </p>

        <div class={["pill", phase()]}>
          <div class="pill-listening">
            <i class="dot" />
            <div class="wave">
              <For each={levels()} keyed={false}>
                {(level, i) => (
                  <i style={{ height: `${3 + level() * 26}px`, opacity: 0.3 + (0.7 * i) / BARS }} />
                )}
              </For>
            </div>
            <span class="time">0:{String(seconds()).padStart(2, "0")}</span>
          </div>
          <div class="pill-hearing">
            <div class="wave small">
              {Array.from({ length: 10 }, (_, i) => (
                <i style={{ "animation-delay": `${i * -110}ms` }} />
              ))}
            </div>
            <span>Hearing</span>
          </div>
          <div class="pill-done">
            {icons.check(14)}
            <span>Typed {TYPED_WORDS} words</span>
          </div>
        </div>
      </div>

      <ol class="steps">
        {steps.map((item, i) => (
          <li class={{ active: step() === i }}>
            <button type="button" onClick={() => jump(i)}>
              <span class="track">
                <i style={{ transform: `scaleX(${progress(i)})` }} />
              </span>
              <span class="step-title">{item.title}</span>
              <span class="step-body">{item.body}</span>
            </button>
          </li>
        ))}
      </ol>
    </div>
  );
}
