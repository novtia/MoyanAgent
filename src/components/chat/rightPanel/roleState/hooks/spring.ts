/** Critically-ish-damped spring for list reorder physics. */

export interface SpringConfig {
  stiffness: number;
  damping: number;
  mass: number;
  /** Rest epsilon for position + velocity. */
  restDelta: number;
}

export const DEFAULT_SPRING: SpringConfig = {
  stiffness: 280,
  damping: 26,
  mass: 1,
  restDelta: 0.15,
};

export interface SpringState {
  current: number;
  target: number;
  velocity: number;
}

export function createSpring(initial = 0): SpringState {
  return { current: initial, target: initial, velocity: 0 };
}

export function prefersReducedMotion(): boolean {
  if (typeof window === "undefined" || !window.matchMedia) return false;
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

/** Integrate one spring step. `dt` in seconds. Returns true if still moving. */
export function stepSpring(
  state: SpringState,
  dt: number,
  config: SpringConfig = DEFAULT_SPRING,
): boolean {
  if (prefersReducedMotion()) {
    state.current = state.target;
    state.velocity = 0;
    return false;
  }
  const { stiffness, damping, mass, restDelta } = config;
  // Clamp dt to avoid spiral after tab switch
  const t = Math.min(Math.max(dt, 0), 0.048);
  const force = -stiffness * (state.current - state.target) - damping * state.velocity;
  const accel = force / mass;
  state.velocity += accel * t;
  state.current += state.velocity * t;
  if (
    Math.abs(state.current - state.target) < restDelta &&
    Math.abs(state.velocity) < restDelta
  ) {
    state.current = state.target;
    state.velocity = 0;
    return false;
  }
  return true;
}

export function setSpringTarget(state: SpringState, target: number, hard = false) {
  state.target = target;
  if (hard || prefersReducedMotion()) {
    state.current = target;
    state.velocity = 0;
  }
}
