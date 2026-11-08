import type { Step } from "@/components/Demo";

/** The playground waits this long after a tab's animations finish before moving on. */
export const HOLD_MS = 3500;
/** The shortest a tab stays up, however little it animates. */
export const MIN_TAB_MS = 8000;

/** About how long `Demo` takes to play a script (typing speeds mirror the ones in Demo.tsx). */
export function scriptMs(steps: Step[]): number {
  let ms = 500;
  for (const step of steps) {
    if ("cmd" in step) ms += step.cmd.length * 24 + 450;
    else if ("agent" in step) ms += step.agent.length * 14 + 450;
    else if ("out" in step) ms += step.out.length * 60;
    else if ("edit" in step) ms += step.from.length * 40 + step.to.length * 65 + 250;
    else if ("insert" in step) ms += step.text.length * 32 + 250;
    else if ("pause" in step) ms += step.pause;
    else ms += 120;
  }
  return ms;
}

/** How long a tab should stay up: until its slowest pane finishes, then a pause to read. */
export function tabMs(...panes: number[]): number {
  return Math.max(MIN_TAB_MS, Math.max(...panes) + HOLD_MS);
}
