// Two small scroll effects, written here so the site needs no animation library.

const calm = () => window.matchMedia("(prefers-reduced-motion: reduce)").matches;

/**
 * Ease the page toward where the wheel asked it to go, instead of jumping there.
 * Touch screens, keyboards and the scrollbar keep their own scrolling.
 */
export function smoothWheel() {
  if (calm() || !window.matchMedia("(pointer: fine)").matches) return () => {};

  let target = window.scrollY;
  let current = window.scrollY;
  let frame = 0;
  let last = 0;

  const stop = () => {
    cancelAnimationFrame(frame);
    frame = 0;
  };
  const step = (now: number) => {
    // The same glide at 60 Hz and 120 Hz.
    const dt = Math.min(0.05, (now - last) / 1000);
    last = now;
    current += (target - current) * (1 - Math.exp(-dt * 9.5));
    if (Math.abs(target - current) < 0.4) current = target;
    window.scrollTo(0, current);
    frame = current === target ? 0 : requestAnimationFrame(step);
  };
  const wheel = (event: WheelEvent) => {
    // Leave pinch-zoom and sideways scrolling alone.
    if (event.ctrlKey || event.metaKey || Math.abs(event.deltaX) > Math.abs(event.deltaY)) return;
    event.preventDefault();
    if (!frame) target = current = window.scrollY;
    const unit = event.deltaMode === 1 ? 32 : event.deltaMode === 2 ? window.innerHeight : 1;
    const end = document.documentElement.scrollHeight - window.innerHeight;
    target = Math.max(0, Math.min(end, target + event.deltaY * unit));
    if (!frame) {
      last = performance.now();
      frame = requestAnimationFrame(step);
    }
  };

  window.addEventListener("wheel", wheel, { passive: false });
  // Anything else that scrolls takes over at once.
  window.addEventListener("keydown", stop);
  window.addEventListener("pointerdown", stop);
  return () => {
    stop();
    window.removeEventListener("wheel", wheel);
    window.removeEventListener("keydown", stop);
    window.removeEventListener("pointerdown", stop);
  };
}

/** Fade each `.reveal` element up into place the first time it scrolls into view. */
export function revealOnScroll() {
  const marked = document.querySelectorAll(".reveal");
  if (calm() || !("IntersectionObserver" in window)) {
    marked.forEach((el) => el.classList.add("in"));
    return () => {};
  }
  const watcher = new IntersectionObserver(
    (entries) => {
      for (const entry of entries) {
        if (!entry.isIntersecting) continue;
        entry.target.classList.add("in");
        watcher.unobserve(entry.target);
      }
    },
    { threshold: 0.08, rootMargin: "0px 0px -6% 0px" },
  );
  marked.forEach((el) => watcher.observe(el));
  return () => watcher.disconnect();
}
