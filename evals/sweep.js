// Capability sweep (PLAN.md T24.1): one deterministic code-mode program, no LLM.
// The harness binds ROWS (every registry row: [id, example, setup ops]) and SAMPLE (a package path).
// 1. Every row applies (as a dry run, after its setup) to the sample.
// 2. A real batch layers looks, keys, transitions, shapes, templates and recipes together.
// 3. Undoing the session restores the exact starting timeline.
const opened = splice.project.info({ path: SAMPLE });
splice.session.start({});
const before = JSON.stringify(splice.timeline.snapshot({ detail: "full" }).spine);
const failures = [];
let applied = 0;
for (const [id, example, setup] of ROWS) {
  try {
    splice.edit.apply({ ifRevision: opened.revision, dryRun: true, ops: [...setup, example] });
    applied += 1;
  } catch (e) {
    failures.push(`${id}: ${e.code} ${e.message}`);
  }
}
const batch = splice.edit.apply({
  ifRevision: opened.revision,
  ops: [
    { cmd: "param.keys", target: "clp_take", path: "transform.scale",
      keys: [{ at: 0, value: 1, ease: "inOut" }, { at: 60, value: 1.4 }] },
    { cmd: "effect.add", target: "project", type: "color", params: { saturation: 1.2, contrast: 1.1 } },
    { cmd: "mask.add", item: "clp_take", mode: "pixelate", x: 0.7, y: 0.05, w: 0.25, h: 0.08 },
    { cmd: "transition.add", item: "clp_end", kind: "dip", length: 12 },
    { cmd: "generator.add", shape: "arrow", at: 200, length: 45, stroke: "#ff3b30", strokeWidth: 0.01 },
    { cmd: "style.set", font: "Avenir Next", color: "#1e3a8a", color2: "#0f172a", accent: "#f59e0b" },
    { cmd: "template.add", name: "lower-third", at: 150, fields: { title: "Ada", subtitle: "Engineer" } },
    { cmd: "effect.add", target: "ovl_hook", type: "animate", params: { preset: "rise" } },
    { cmd: "param.keys", target: "clp_take", path: "speed.rate",
      keys: [{ at: 0, value: 1 }, { at: 60, value: 1.5 }, { at: 120, value: 1 }] },
    { cmd: "crop.keys", item: "clp_open",
      keys: [{ at: 0, x: 0, y: 0, w: 1, h: 1, ease: "inOut" }, { at: 60, x: 0.1, y: 0.1, w: 0.6, h: 0.6 }] },
    { cmd: "cut.pauses", min: 18, keep: 4 },
    { cmd: "captions.translate", lane: "lan_cap", language: "es",
      texts: ["Hola", "Mira", "Abre", "Elige", "Prueba", "Listo", "Aquí", "Guarda", "Comparte", "Gracias"] },
  ],
});
const undone = splice.history.undo({ scope: "session", ifRevision: batch.revision });
const after = JSON.stringify(splice.timeline.snapshot({ detail: "full" }).spine);
return {
  rows: ROWS.length,
  applied,
  failures,
  batchChanges: batch.changes.length,
  restored: before === after,
  revision: undone.revision,
};
