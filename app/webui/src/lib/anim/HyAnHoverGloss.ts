import type { ActionReturn } from "svelte/action";

const MAX_TILT = 1;
const SPRING = "transform 0.4s cubic-bezier(0.22, 1, 0.36, 1)";

function createOverlay(node: HTMLElement): HTMLDivElement {
  const overlay = globalThis.document.createElement("div");
  overlay.style.cssText = [
    "position:absolute",
    "inset:0",
    "border-radius:inherit",
    "pointer-events:none",
    "z-index:10",
    "opacity:0",
    "transition:opacity 0.3s ease",
  ].join(";");
  node.appendChild(overlay);
  return overlay;
}

/** Adds Hyphen's pointer-driven tilt and gloss to metric cards. */
export function HyAnHoverGloss(node: HTMLElement): ActionReturn {
  node.style.transition = SPRING;
  node.style.willChange = "transform";
  node.style.transformStyle = "preserve-3d";

  const overlay = createOverlay(node);

  function handleMove(event: MouseEvent): void {
    const rect = node.getBoundingClientRect();
    const normalizedX = ((event.clientX - rect.left) / rect.width) * 2 - 1;
    const normalizedY = ((event.clientY - rect.top) / rect.height) * 2 - 1;
    const percentX = ((event.clientX - rect.left) / rect.width) * 100;
    const percentY = ((event.clientY - rect.top) / rect.height) * 100;

    const rotateY = -normalizedY * MAX_TILT;
    const rotateX = normalizedX * MAX_TILT;

    node.style.transition = "none";
    node.style.transform = `perspective(800px) rotateX(${String(rotateY)}deg) rotateY(${String(rotateX)}deg)`;

    overlay.style.opacity = "1";
    overlay.style.background =
      `radial-gradient(circle at ${String(percentX)}% ${String(percentY)}%, ` +
      "rgba(255,255,255,0.04) 0%, transparent 90%)";
  }

  function handleLeave(): void {
    node.style.transition = SPRING;
    node.style.transform = "perspective(800px) rotateX(0) rotateY(0)";
    overlay.style.opacity = "0";
  }

  node.addEventListener("mousemove", handleMove);
  node.addEventListener("mouseleave", handleLeave);

  return {
    destroy(): void {
      node.removeEventListener("mousemove", handleMove);
      node.removeEventListener("mouseleave", handleLeave);
      overlay.remove();
    },
  };
}
