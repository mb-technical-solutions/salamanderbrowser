/**
 * Shared physics for the Salamander logo easter eggs: clones an element into
 * a fixed-position layer, drops it with gravity onto the floor, then has it
 * crawl along the floor with an undulating gait until it walks off an edge —
 * grabbable and throwable along the way. Used by the rail logo (5-click
 * trigger) and the About dialog flywheel escape.
 */

const GRAVITY = 2200;
const CRAWL_SPEED = 220; // px/s horizontal walk speed
const BOB_AMPLITUDE = 5; // px vertical bob while crawling
const BOB_FREQUENCY = 7; // rad/s
const SWAY_DEGREES = 8; // body sway while crawling

export interface SalamanderLaunchOptions {
  /** Initial horizontal velocity in px/s. Defaults to a rightward crawl. */
  initialVX?: number;
  /** Initial vertical velocity in px/s (negative = upward). */
  initialVY?: number;
  /** Called once the clone has walked off the screen and been removed. */
  onExit?: () => void;
}

/**
 * Launch a physics clone of `el`. The source element is hidden (visibility)
 * and stays hidden — the caller decides when/if to restore it. Returns a
 * cancel function that removes the clone and stops the sim.
 */
export function launchSalamanderClone(
  el: HTMLElement,
  options: SalamanderLaunchOptions = {},
): () => void {
  // getBoundingClientRect measures the *transformed* box. A caller that rotates
  // the element before launching (the About dialog spins it up to escape
  // velocity) would otherwise hand us the rotated bounding box — so suppress
  // the transform for the measurement to get the layout box, then put it back.
  const previousTransform = el.style.transform;
  if (previousTransform) {
    el.style.transform = "none";
  }
  const rect = el.getBoundingClientRect();
  if (previousTransform) {
    el.style.transform = previousTransform;
  }
  const startX = rect.left;
  const startY = rect.top;

  const clone = el.cloneNode(true) as HTMLElement;
  clone.style.position = "fixed";
  clone.style.left = `${startX}px`;
  clone.style.top = `${startY}px`;
  clone.style.zIndex = "9999";
  clone.style.margin = "0";
  // The escaped salamander is a toy: it can be grabbed 1:1 and thrown,
  // inheriting the pointer's release velocity.
  clone.style.pointerEvents = "auto";
  clone.style.cursor = "grab";
  clone.style.touchAction = "none";
  document.body.appendChild(clone);
  el.style.visibility = "hidden";

  let x = 0;
  let y = 0;
  let vy = options.initialVY ?? -500;
  let vx = options.initialVX ?? CRAWL_SPEED;
  let direction: 1 | -1 = vx >= 0 ? 1 : -1;
  let phase: "falling" | "crawling" = "falling";
  let crawlTime = 0;
  let lastTime = performance.now();
  let grabbed = false;
  let grabDX = 0;
  let grabDY = 0;
  let animFrame = 0;
  let cancelled = false;
  // Recent pointer positions (≤100ms) for release-velocity estimation.
  let history: { t: number; x: number; y: number }[] = [];

  const applyTransform = () => {
    const bob =
      phase === "crawling"
        ? Math.abs(Math.sin(crawlTime * BOB_FREQUENCY)) * -BOB_AMPLITUDE
        : 0;
    const sway =
      phase === "crawling"
        ? Math.sin(crawlTime * BOB_FREQUENCY) * SWAY_DEGREES
        : 0;
    const flip = direction === -1 ? -1 : 1;
    clone.style.transform =
      `translate(${x}px, ${y + bob}px) scaleX(${flip}) rotate(${sway}deg)`;
  };

  const onPointerDown = (e: PointerEvent) => {
    if (grabbed) return;
    grabbed = true;
    clone.setPointerCapture(e.pointerId);
    clone.style.cursor = "grabbing";
    // Respect where the salamander was grabbed — no snap to center.
    grabDX = e.clientX - (startX + x);
    grabDY = e.clientY - (startY + y);
    vx = 0;
    vy = 0;
    history = [{ t: performance.now(), x, y }];
  };

  const onPointerMove = (e: PointerEvent) => {
    if (!grabbed) return;
    x = e.clientX - grabDX - startX;
    y = e.clientY - grabDY - startY;
    const now = performance.now();
    history.push({ t: now, x, y });
    while (history.length > 1 && now - history[0].t > 100) history.shift();
    applyTransform();
  };

  const onPointerUp = (e: PointerEvent) => {
    if (!grabbed) return;
    grabbed = false;
    clone.style.cursor = "grab";
    try {
      clone.releasePointerCapture(e.pointerId);
    } catch {
      // capture already gone
    }
    // Velocity handoff: the sim continues at the finger's speed so a flick
    // throws the salamander instead of dropping it.
    const now = performance.now();
    const oldest = history[0];
    const dt = oldest ? (now - oldest.t) / 1000 : 0;
    if (oldest && dt > 0.016) {
      vx = (x - oldest.x) / dt;
      vy = (y - oldest.y) / dt;
    }
    direction = vx >= 0 ? 1 : -1;
    phase = "falling";
    history = [];
    lastTime = now;
  };

  clone.addEventListener("pointerdown", onPointerDown);
  clone.addEventListener("pointermove", onPointerMove);
  clone.addEventListener("pointerup", onPointerUp);
  clone.addEventListener("pointercancel", onPointerUp);

  const animate = (time: number) => {
    if (cancelled) return;
    const dt = Math.min((time - lastTime) / 1000, 0.05);
    lastTime = time;

    // Read live so a mid-animation window resize moves the floor.
    const floorY = window.innerHeight;

    if (!grabbed) {
      if (phase === "falling") {
        vy += GRAVITY * dt;
        x += vx * dt;
        y += vy * dt;

        const currentBottom = startY + y + rect.height;
        if (currentBottom >= floorY && vy > 0) {
          // No bounce — it lands on its feet and starts crawling away.
          y = floorY - startY - rect.height;
          phase = "crawling";
          crawlTime = 0;
          vx = CRAWL_SPEED * direction;
          vy = 0;
        }
      } else {
        crawlTime += dt;
        x += vx * dt;
        y = floorY - startY - rect.height;
      }
      applyTransform();
    }

    const offScreenLeft = startX + x + rect.width < -50;
    const offScreenRight = startX + x > window.innerWidth + 50;
    const offScreenBottom = startY + y > floorY + 200;

    if (!grabbed && (offScreenLeft || offScreenRight || offScreenBottom)) {
      clone.remove();
      options.onExit?.();
      return;
    }
    animFrame = requestAnimationFrame(animate);
  };
  animFrame = requestAnimationFrame(animate);

  return () => {
    cancelled = true;
    cancelAnimationFrame(animFrame);
    clone.remove();
  };
}
