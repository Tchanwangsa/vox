import { createSignal, onSettled, Show } from "solid-js";
import {
  closing,
  engines,
  features,
  footer,
  hero,
  install,
  openSource,
  RELEASE_API,
  RELEASES,
  REPO,
  screens,
} from "./copy";
import Demo from "./Demo";
import { GitHubMark, icons, VoxMark } from "./Icons";
import { revealOnScroll, smoothWheel } from "./scroll";

type Release = { version: string; dmg: string };
type Model = { mark: string; name: string; tint?: string; note?: string };

function Keys() {
  return (
    <span class="keys">
      <span class="sr-only">Control Option Space</span>
      <kbd aria-hidden="true">⌃</kbd>
      <kbd aria-hidden="true">⌥</kbd>
      <kbd aria-hidden="true">Space</kbd>
    </span>
  );
}

/** Copy with `**bold**` marks in it, for the words a reader is hunting for. */
function Rich(props: { text: string }) {
  return <>{props.text.split(/\*\*(.+?)\*\*/g).map((part, i) => (i % 2 ? <strong>{part}</strong> : part))}</>;
}

/** A model maker's logo, drawn in one colour from public/brand. */
function Mark(props: { name: string; tint?: string }) {
  return <i class="mark" style={{ "--mark": `url(/brand/${props.name}.svg)`, color: props.tint }} />;
}

/** The models for one job. Several become a picker that turns over on its own until someone chooses. */
function Models(props: { label: string; list: readonly Model[] }) {
  const [picked, setPicked] = createSignal(0);
  let chosen = false;
  onSettled(() => {
    if (props.list.length < 2 || window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    const timer = window.setInterval(() => {
      if (!chosen && !document.hidden) setPicked((i) => (i + 1) % props.list.length);
    }, 3400);
    return () => window.clearInterval(timer);
  });
  return (
    <Show
      when={props.list.length > 1}
      fallback={
        <span class="model">
          <Mark name={props.list[0].mark} tint={props.list[0].tint} />
          {props.list[0].name}
        </span>
      }
    >
      <div class="picker">
        <div class="segments" role="radiogroup" aria-label={props.label}>
          {props.list.map((model, i) => (
            <button
              type="button"
              role="radio"
              aria-checked={picked() === i ? "true" : "false"}
              class={{ on: picked() === i }}
              onClick={() => {
                chosen = true;
                setPicked(i);
              }}
            >
              <Mark name={model.mark} tint={model.tint} />
              {model.name}
            </button>
          ))}
        </div>
        <span class="picker-note">{props.list[picked()].note}</span>
      </div>
    </Show>
  );
}

function Command(props: { text: string }) {
  const [copied, setCopied] = createSignal(false);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(props.text);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1600);
    } catch {
      // No clipboard access: the command is still there to select by hand.
    }
  };
  return (
    <div class="command">
      <code>{props.text}</code>
      <button type="button" onClick={copy} aria-label={`Copy command: ${props.text}`}>
        <Show when={copied()} fallback={icons.copy(15)}>
          {icons.check(15)}
        </Show>
        <span>{copied() ? "Copied" : "Copy"}</span>
      </button>
    </div>
  );
}

// Where the app's own tabs sit in each screenshot, as percentages of its width,
// so they can be clicked like the real thing.
const TAB_SPOTS = { home: [10.45, 8.86], stats: [19.55, 8.41], engine: [28.18, 9.55], settings: [37.95, 10.57] };

function Screens() {
  const [tab, setTab] = createSignal<string>(screens.tabs[0].id);
  const tabIcons = { home: icons.home, stats: icons.chart, engine: icons.layers, settings: icons.settings };
  const caption = () => screens.tabs.find((item) => item.id === tab())!.caption;
  return (
    <section class="section screens reveal" id="screens">
      <h2>{screens.heading}</h2>

      {/* Small screens get tabs of their own: the ones in the picture are too small to tap. */}
      <div class="segments screen-tabs" role="tablist" aria-label={screens.heading}>
        {screens.tabs.map((item) => (
          <button
            type="button"
            role="tab"
            aria-selected={tab() === item.id ? "true" : "false"}
            class={{ on: tab() === item.id }}
            onClick={() => setTab(item.id)}
          >
            {tabIcons[item.id](15)}
            {item.label}
          </button>
        ))}
      </div>

      <div class="shots">
        <div class="frame">
          {screens.tabs.map((item, i) => (
            <img
              src={`/screens/${item.id}.webp`}
              width="1760"
              height="1772"
              alt={`The ${item.label} page of Vox`}
              class={{ on: tab() === item.id }}
              aria-hidden={tab() === item.id ? "false" : "true"}
              loading={i === 0 ? "eager" : "lazy"}
              decoding="async"
            />
          ))}
          <div class="spots" role="tablist" aria-label={screens.heading}>
            {screens.tabs.map((item) => (
              <button
                type="button"
                role="tab"
                class="spot"
                aria-selected={tab() === item.id ? "true" : "false"}
                aria-label={item.label}
                style={{ left: `${TAB_SPOTS[item.id][0]}%`, width: `${TAB_SPOTS[item.id][1]}%` }}
                onClick={() => setTab(item.id)}
              />
            ))}
          </div>
          {/* Home's own "See stats" button goes where it says. */}
          <Show when={tab() === "home"}>
            <button type="button" class="spot see-stats" aria-label="See stats" onClick={() => setTab("stats")} />
          </Show>
        </div>
        <p class="caption">{caption()}</p>
        <p class="hint">{screens.hint}</p>
      </div>
    </section>
  );
}

export default function App() {
  // The newest release, so the button downloads the disk image itself.
  // Until GitHub answers, or if it doesn't, the button opens the releases page.
  const [release, setRelease] = createSignal<Release>();
  onSettled(() => {
    fetch(RELEASE_API, { headers: { Accept: "application/vnd.github+json" } })
      .then((response) => (response.ok ? response.json() : undefined))
      .then((data) => {
        const dmg = data?.assets?.find((asset: { name: string }) => asset.name.endsWith(".dmg"));
        if (dmg) setRelease({ version: data.tag_name, dmg: dmg.browser_download_url });
      })
      .catch(() => {});
  });
  onSettled(() => {
    const stopWheel = smoothWheel();
    const stopReveal = revealOnScroll();
    return () => {
      stopWheel();
      stopReveal();
    };
  });
  const download = () => release()?.dmg ?? RELEASES;

  const Download = (props: { small?: boolean }) => (
    <a class={["button", "solid", { small: props.small }]} href={download()}>
      {icons.download(props.small ? 15 : 17)}
      {props.small ? "Download" : hero.primary}
    </a>
  );

  return (
    <>
      <header class="top">
        <a class="brand" href="/" aria-label="Vox">
          <VoxMark size={30} />
          <span>Vox</span>
        </a>
        <nav>
          <a class="link" href={REPO}>
            GitHub
          </a>
          <Download small />
        </nav>
      </header>

      <main>
        <section class="hero">
          <h1>
            Hold <Keys /> and speak.
          </h1>
          <p class="lede">{hero.sub}</p>
          <div class="actions">
            <Download />
            <a class="button" href={REPO}>
              <GitHubMark size={16} />
              {hero.secondary}
            </a>
          </div>
          <p class="note">
            {hero.note}
            <Show when={release()}>{(found) => <span> {found().version}.</span>}</Show>
          </p>
        </section>

        <Demo />

        <section class="section reveal">
          <div class="features">
            {features.map((item) => (
              <article>
                <span class="icon">{icons[item.icon](22)}</span>
                <h3>{item.title}</h3>
                <p>{item.body}</p>
              </article>
            ))}
          </div>
        </section>

        <Screens />

        <section class="section engines reveal" id="engines">
          <h2>{engines.heading}</h2>
          <p class="lede">{engines.sub}</p>
          <div class="cards">
            {engines.cards.map((card) => (
              <article>
                <h3>
                  <Show when={card.mark} fallback={icons.monitor(19)}>
                    <Mark name={card.mark!} tint={card.tint} />
                  </Show>
                  {card.name}
                </h3>
                <p>{card.body}</p>
                <dl>
                  <div class={{ pick: card.hears.length > 1 }}>
                    <dt>Hears with</dt>
                    <dd>
                      <Models label="Hears with" list={card.hears} />
                    </dd>
                  </div>
                  <div>
                    <dt>Tidies with</dt>
                    <dd>
                      <Models label="Tidies with" list={card.tidies} />
                    </dd>
                  </div>
                </dl>
              </article>
            ))}
          </div>
        </section>

        <section class="section install reveal" id="install">
          <h2>{install.heading}</h2>
          <ol>
            <li>
              <p>
                <Rich text={install.steps[0]} />
              </p>
              <Download small />
            </li>
            <li>
              <p>
                <Rich text={install.steps[1]} />
              </p>
              <Command text={install.unblock} />
            </li>
            <li>
              <p>
                <Rich text={install.steps[2]} />
              </p>
              <span class="chip">
                {icons.settings(15)}
                {install.settingsPath}
              </span>
            </li>
          </ol>
          <div class="aside">
            <p>{install.localModelsNote}</p>
            <Command text={install.brew} />
          </div>
        </section>

        <section class="section source reveal">
          <div>
            <h2>{openSource.heading}</h2>
            <p class="lede">{openSource.body}</p>
            <a class="more" href={REPO}>
              <GitHubMark size={17} />
              {openSource.cta}
              {icons.arrow(16)}
            </a>
          </div>
          <div class="terminal" role="img" aria-label={`Build from source: ${openSource.commands.join(", then ")}`}>
            <div class="terminal-bar">
              <i />
              <i />
              <i />
              <span>zsh</span>
            </div>
            <pre aria-hidden="true">
              {openSource.commands.map((line) => (
                <>
                  <span class="prompt">$ </span>
                  {line}
                  {"\n"}
                </>
              ))}
              <span class="comment"># {openSource.comment}</span>
            </pre>
          </div>
        </section>

        <section class="closing reveal">
          <VoxMark size={56} />
          <h2>{closing.headline}</h2>
          <p class="lede">{closing.sub}</p>
          <div class="actions">
            <Download />
          </div>
        </section>
      </main>

      <footer class="bottom">
        <p>{footer.line}</p>
        <nav>
          <a href={REPO}>GitHub</a>
          <a href={RELEASES}>Releases</a>
          <a href={`${REPO}/blob/main/LICENSE`}>License</a>
        </nav>
        <p class="credit">
          Made by <a href={footer.author.url}>{footer.author.name}</a>
        </p>
        <p class="marks">{footer.marks}</p>
      </footer>
    </>
  );
}
