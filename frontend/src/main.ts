import { createAppShellState } from "./app-shell.ts";

const app = document.querySelector<HTMLDivElement>("#app");

if (app) {
  const state = createAppShellState();
  app.innerHTML = `
    <section class="app-shell">
      <h1>meetnote</h1>
      <p>Developer skeleton ready.</p>
      <ul>
        ${state.sections.map((section) => `<li>${section}</li>`).join("")}
      </ul>
    </section>
  `;
}
