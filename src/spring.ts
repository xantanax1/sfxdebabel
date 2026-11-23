export interface SpringOptions {
  initial?: number;
  stiffness?: number;
  damping?: number;
  mass?: number;
  precision?: number;
  onUpdate?: (value: number) => void;
  onSettled?: () => void;
}

export function prefersReducedMotion(): boolean {
  return (
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

function clamp(v: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, v));
}

export class Spring {
  value: number;
  velocity = 0;
  target: number;
  stiffness: number;
  damping: number;
  mass: number;
  precision: number;
  private onUpdate?: (value: number) => void;
  private onSettled?: () => void;

  constructor(options: SpringOptions = {}) {
    const initial = options.initial ?? 0;
    this.value = initial;
    this.target = initial;
    this.stiffness = options.stiffness ?? 260;
    this.damping = options.damping ?? 26;
    this.mass = options.mass ?? 1;
    this.precision = options.precision ?? 0.001;
    this.onUpdate = options.onUpdate;
    this.onSettled = options.onSettled;
  }

  setTarget(target: number, scheduler: SpringScheduler = sharedScheduler): void {
    this.target = target;
    if (prefersReducedMotion()) {
      this.snap(target);
      this.onSettled?.();
      return;
    }
    if (this.isSettled()) {
      this.value = target;
      this.velocity = 0;
      this.onUpdate?.(target);
      this.onSettled?.();
      return;
    }
    scheduler.add(this);
  }

  impulse(deltaVelocity: number, scheduler: SpringScheduler = sharedScheduler): void {
    if (prefersReducedMotion()) return;
    this.velocity += deltaVelocity;
    scheduler.add(this);
  }

  snap(value: number): void {
    this.value = value;
    this.target = value;
    this.velocity = 0;
    this.onUpdate?.(value);
  }

  isSettled(): boolean {
    return (
      Math.abs(this.target - this.value) <= this.precision &&
      Math.abs(this.velocity) <= this.precision
    );
  }

  step(dt: number): boolean {
    const maxStep = 1 / 120;
    let remaining = Math.min(0.05, Math.max(0.001, dt));

    while (remaining > 0) {
      const h = Math.min(maxStep, remaining);
      const forceSpring = -this.stiffness * (this.value - this.target);
      const forceDamp = -this.damping * this.velocity;
      const accel = (forceSpring + forceDamp) / this.mass;
      this.velocity += accel * h;
      this.value += this.velocity * h;
      remaining -= h;
    }

    if (this.isSettled()) {
      this.value = this.target;
      this.velocity = 0;
      this.onUpdate?.(this.value);
      this.onSettled?.();
      return false;
    }

    this.onUpdate?.(this.value);
    return true;
  }
}

export class SpringScheduler {
  private active = new Set<Spring>();
  private rafId: number | null = null;
  private lastTime = 0;

  add(spring: Spring): void {
    this.active.add(spring);
    if (this.rafId === null) {
      this.lastTime = performance.now();
      this.rafId = requestAnimationFrame(this.tick);
    }
  }

  remove(spring: Spring): void {
    this.active.delete(spring);
    if (this.active.size === 0 && this.rafId !== null) {
      cancelAnimationFrame(this.rafId);
      this.rafId = null;
    }
  }

  private tick = (now: number): void => {
    const dt = (now - this.lastTime) / 1000;
    this.lastTime = now;

    const current = Array.from(this.active);
    for (const spring of current) {
      const keepGoing = spring.step(dt);
      if (!keepGoing) {
        this.active.delete(spring);
      }
    }

    if (this.active.size > 0) {
      this.rafId = requestAnimationFrame(this.tick);
    } else {
      this.rafId = null;
    }
  };
}

export const sharedScheduler = new SpringScheduler();

const boundSpecular = new WeakSet<HTMLElement>();
const boundTilt = new WeakSet<HTMLElement>();

export interface SpecularOptions {
  maxOpacity?: number;
}

export function bindSpecularSurface(
  el: HTMLElement,
  options: SpecularOptions = {}
): () => void {
  if (boundSpecular.has(el)) {
    return () => {};
  }
  boundSpecular.add(el);
  el.classList.add("glass-specular");

  const maxOpacity = options.maxOpacity ?? 1;

  const mx = new Spring({
    initial: 50,
    stiffness: 340,
    damping: 30,
    precision: 0.05,
    onUpdate: (v) => {
      if (el.isConnected) {
        el.style.setProperty("--mx", `${v.toFixed(2)}%`);
      }
    },
  });

  const my = new Spring({
    initial: 50,
    stiffness: 340,
    damping: 30,
    precision: 0.05,
    onUpdate: (v) => {
      if (el.isConnected) {
        el.style.setProperty("--my", `${v.toFixed(2)}%`);
      }
    },
  });

  const specO = new Spring({
    initial: 0,
    stiffness: 250,
    damping: 26,
    precision: 0.004,
    onUpdate: (v) => {
      if (el.isConnected) {
        el.style.setProperty("--spec-o", v.toFixed(3));
      }
    },
  });

  const coordsFromEvent = (ev: PointerEvent): { x: number; y: number } => {
    const rect = el.getBoundingClientRect();
    const w = Math.max(1, rect.width);
    const h = Math.max(1, rect.height);
    return {
      x: clamp(((ev.clientX - rect.left) / w) * 100, 0, 100),
      y: clamp(((ev.clientY - rect.top) / h) * 100, 0, 100),
    };
  };

  const onEnter = (ev: PointerEvent) => {
    const { x, y } = coordsFromEvent(ev);
    if (specO.value < 0.03) {
      mx.snap(x);
      my.snap(y);
    } else {
      mx.setTarget(x);
      my.setTarget(y);
    }
    specO.setTarget(maxOpacity);
  };

  const onMove = (ev: PointerEvent) => {
    const { x, y } = coordsFromEvent(ev);
    mx.setTarget(x);
    my.setTarget(y);
    if (specO.target !== maxOpacity) {
      specO.setTarget(maxOpacity);
    }
  };

  const onLeave = () => {
    specO.setTarget(0);
  };

  el.addEventListener("pointerenter", onEnter);
  el.addEventListener("pointermove", onMove);
  el.addEventListener("pointerleave", onLeave);
  el.addEventListener("pointercancel", onLeave);

  return () => {
    el.removeEventListener("pointerenter", onEnter);
    el.removeEventListener("pointermove", onMove);
    el.removeEventListener("pointerleave", onLeave);
    el.removeEventListener("pointercancel", onLeave);
    sharedScheduler.remove(mx);
    sharedScheduler.remove(my);
    sharedScheduler.remove(specO);
  };
}

export interface TiltParallaxOptions {
  maxRotateX?: number;
  maxRotateY?: number;
  liftZ?: number;
  pressZ?: number;
  parallaxPx?: number;
  flattenOnPress?: boolean;
}

export function bindTiltParallax(
  el: HTMLElement,
  options: TiltParallaxOptions = {}
): () => void {
  if (boundTilt.has(el)) {
    return () => {};
  }
  boundTilt.add(el);

  const maxRotateX = options.maxRotateX ?? 1.8;
  const maxRotateY = options.maxRotateY ?? 1.0;
  const liftZ = options.liftZ ?? 4;
  const pressZ = options.pressZ ?? -1;
  const parallaxPx = options.parallaxPx ?? 3.0;
  const flattenOnPress = options.flattenOnPress ?? true;

  let isHovered = false;
  let isPressed = false;
  let lastNx = 0;
  let lastNy = 0;

  const rx = new Spring({
    initial: 0,
    stiffness: 250,
    damping: 25,
    precision: 0.005,
    onUpdate: (v) => {
      if (el.isConnected) el.style.setProperty("--rx", `${v.toFixed(3)}deg`);
    },
  });

  const ry = new Spring({
    initial: 0,
    stiffness: 250,
    damping: 25,
    precision: 0.005,
    onUpdate: (v) => {
      if (el.isConnected) el.style.setProperty("--ry", `${v.toFixed(3)}deg`);
    },
  });

  const tz = new Spring({
    initial: 0,
    stiffness: 290,
    damping: 26,
    precision: 0.02,
    onUpdate: (v) => {
      if (el.isConnected) el.style.setProperty("--tz", `${v.toFixed(2)}px`);
    },
  });

  const px = new Spring({
    initial: 0,
    stiffness: 240,
    damping: 24,
    precision: 0.01,
    onUpdate: (v) => {
      if (el.isConnected) el.style.setProperty("--px", `${v.toFixed(2)}px`);
    },
  });

  const py = new Spring({
    initial: 0,
    stiffness: 240,
    damping: 24,
    precision: 0.01,
    onUpdate: (v) => {
      if (el.isConnected) el.style.setProperty("--py", `${v.toFixed(2)}px`);
    },
  });

  const applyTargets = () => {
    if (prefersReducedMotion()) {
      rx.snap(0);
      ry.snap(0);
      tz.snap(0);
      px.snap(0);
      py.snap(0);
      return;
    }

    if (isPressed && flattenOnPress) {
      rx.setTarget(0);
      ry.setTarget(0);
      tz.setTarget(pressZ);
      px.setTarget(0);
      py.setTarget(0);
      return;
    }

    if (!isHovered) {
      rx.setTarget(0);
      ry.setTarget(0);
      tz.setTarget(0);
      px.setTarget(0);
      py.setTarget(0);
      return;
    }

    rx.setTarget(-lastNy * maxRotateX);
    ry.setTarget(lastNx * maxRotateY);
    tz.setTarget(liftZ);
    px.setTarget(lastNx * parallaxPx);
    py.setTarget(lastNy * parallaxPx);
  };

  const updateNormFromEvent = (ev: PointerEvent) => {
    const rect = el.getBoundingClientRect();
    const w = Math.max(1, rect.width);
    const h = Math.max(1, rect.height);
    lastNx = clamp(((ev.clientX - rect.left) / w - 0.5) * 2, -1, 1);
    lastNy = clamp(((ev.clientY - rect.top) / h - 0.5) * 2, -1, 1);
  };

  const onEnter = (ev: PointerEvent) => {
    isHovered = true;
    updateNormFromEvent(ev);
    applyTargets();
  };

  const onMove = (ev: PointerEvent) => {
    isHovered = true;
    updateNormFromEvent(ev);
    applyTargets();
  };

  const onDown = (ev: PointerEvent) => {
    if (ev.button !== 0) return;
    isPressed = true;
    applyTargets();
  };

  const onWindowUp = () => {
    if (!isPressed) return;
    isPressed = false;
    applyTargets();
  };

  const onLeave = () => {
    isHovered = false;
    isPressed = false;
    applyTargets();
  };

  el.addEventListener("pointerenter", onEnter);
  el.addEventListener("pointermove", onMove);
  el.addEventListener("pointerdown", onDown);
  el.addEventListener("pointerleave", onLeave);
  el.addEventListener("pointercancel", onLeave);
  window.addEventListener("pointerup", onWindowUp);
  window.addEventListener("pointercancel", onWindowUp);

  return () => {
    el.removeEventListener("pointerenter", onEnter);
    el.removeEventListener("pointermove", onMove);
    el.removeEventListener("pointerdown", onDown);
    el.removeEventListener("pointerleave", onLeave);
    el.removeEventListener("pointercancel", onLeave);
    window.removeEventListener("pointerup", onWindowUp);
    window.removeEventListener("pointercancel", onWindowUp);
    sharedScheduler.remove(rx);
    sharedScheduler.remove(ry);
    sharedScheduler.remove(tz);
    sharedScheduler.remove(px);
    sharedScheduler.remove(py);
  };
}

export function bindScrollGlass(scrollEl: HTMLElement, targetEl: HTMLElement): () => void {
  const depthSpring = new Spring({
    initial: 0,
    stiffness: 185,
    damping: 24,
    precision: 0.002,
    onUpdate: (depth) => {
      if (!targetEl.isConnected) return;
      targetEl.style.setProperty("--scroll-depth", depth.toFixed(4));
      targetEl.style.setProperty("--scroll-blur", `${(depth * 8).toFixed(2)}px`);
      targetEl.style.setProperty("--scroll-sat", `${(depth * 24).toFixed(1)}%`);
    },
  });

  const onScroll = () => {
    const top = scrollEl.scrollTop;
    const norm = clamp(top / 160, 0, 1);
    scrollEl.classList.toggle("is-scrolled", top > 4);
    targetEl.classList.toggle("is-scrolled", top > 4);
    depthSpring.setTarget(norm);
  };

  scrollEl.addEventListener("scroll", onScroll, { passive: true });
  onScroll();

  return () => {
    scrollEl.removeEventListener("scroll", onScroll);
    sharedScheduler.remove(depthSpring);
  };
}

export interface ScopeParallaxHandle {
  getOffset: () => { x: number; y: number };
  destroy: () => void;
}

export function bindScopeParallax(
  canvasEl: HTMLElement,
  onRedraw: () => void
): ScopeParallaxHandle {
  let redrawScheduled = false;
  const scheduleRedraw = () => {
    if (redrawScheduled) return;
    redrawScheduled = true;
    queueMicrotask(() => {
      redrawScheduled = false;
      if (canvasEl.isConnected) {
        onRedraw();
      }
    });
  };

  const px = new Spring({
    initial: 0,
    stiffness: 200,
    damping: 23,
    precision: 0.002,
    onUpdate: scheduleRedraw,
  });

  const py = new Spring({
    initial: 0,
    stiffness: 200,
    damping: 23,
    precision: 0.002,
    onUpdate: scheduleRedraw,
  });

  const onMove = (ev: PointerEvent) => {
    if (prefersReducedMotion()) return;
    if ((ev.buttons & 1) !== 0) {
      px.setTarget(0);
      py.setTarget(0);
      return;
    }
    const rect = canvasEl.getBoundingClientRect();
    const w = Math.max(1, rect.width);
    const h = Math.max(1, rect.height);
    const nx = clamp(((ev.clientX - rect.left) / w - 0.5) * 2, -1, 1);
    const ny = clamp(((ev.clientY - rect.top) / h - 0.5) * 2, -1, 1);
    px.setTarget(nx);
    py.setTarget(ny);
  };

  const onDown = (ev: PointerEvent) => {
    if (ev.button !== 0) return;
    px.setTarget(0);
    py.setTarget(0);
  };

  const onLeave = () => {
    px.setTarget(0);
    py.setTarget(0);
  };

  canvasEl.addEventListener("pointermove", onMove);
  canvasEl.addEventListener("pointerdown", onDown);
  canvasEl.addEventListener("pointerleave", onLeave);
  canvasEl.addEventListener("pointercancel", onLeave);

  return {
    getOffset: () => ({ x: px.value, y: py.value }),
    destroy: () => {
      canvasEl.removeEventListener("pointermove", onMove);
      canvasEl.removeEventListener("pointerdown", onDown);
      canvasEl.removeEventListener("pointerleave", onLeave);
      canvasEl.removeEventListener("pointercancel", onLeave);
      sharedScheduler.remove(px);
      sharedScheduler.remove(py);
    },
  };
}
