// Renders states.json. Every colour and measurement comes from that file,
// which the Rust prints; this file decides structure and nothing else.

const state = { data: null, appearance: "light", index: 0, wallpaper: true, preset: "gnome" };

async function load() {
  const response = await fetch("states.json");
  state.data = await response.json();

  const select = document.getElementById("state");
  state.data.states.forEach((s, i) => {
    const option = document.createElement("option");
    option.value = String(i);
    option.textContent = s.name;
    select.append(option);
  });

  select.addEventListener("change", () => {
    state.index = Number(select.value);
    render();
  });

  const presets = document.getElementById("preset");
  for (const p of state.data.presets) {
    const option = document.createElement("option");
    option.value = p.name;
    option.textContent = p.name;
    presets.append(option);
  }
  presets.value = state.preset;
  presets.addEventListener("change", (event) => {
    state.preset = event.target.value;
    render();
  });
  document.getElementById("appearance").addEventListener("change", (event) => {
    state.appearance = event.target.value;
    render();
  });
  document.getElementById("wallpaper").addEventListener("change", (event) => {
    state.wallpaper = event.target.checked;
    render();
  });

  render();
}

/// The preset in force, or `gnome` if the name does not resolve.
///
/// Mirrors `preset::resolve`'s fallback rather than throwing: a surrogate that
/// blanks on an unknown name is less useful than one that draws the default,
/// which is what the launcher itself would do.
function activePreset() {
  const presets = state.data.presets ?? [];
  return presets.find((p) => p.name === state.preset) ?? presets[0];
}

function applyTokens() {
  const { fontStack, appearances } = state.data;
  const preset = activePreset();
  // The preset's geometry, not the file's top-level one. They are equal for
  // `gnome` by construction -- the Rust resolves that preset to
  // `design::GEOMETRY` itself -- so reading the preset here is what makes the
  // other three visible at all.
  const geometry = preset ? preset.geometry : state.data.geometry;
  const palette = appearances.find((a) => a.name === state.appearance);
  const screen = document.getElementById("screen");
  const root = document.documentElement.style;
  for (const [name, value] of Object.entries(state.data.panelMetrics)) {
    root.setProperty(`--panel-${name}`, `${value}px`);
  }

  root.setProperty("--surface", palette.surface);
  root.setProperty("--field", palette.field);
  root.setProperty("--text", palette.text);
  root.setProperty("--muted", palette.muted);
  root.setProperty("--selection", palette.selection);
  root.setProperty("--selection-text", palette.selectionText);
  root.setProperty("--border", palette.border);
  root.setProperty("--accent", palette.accent);
  root.setProperty("--backdrop", palette.backdrop);
  root.setProperty("--backdrop-alpha", String(palette.backdropAlpha));

  root.setProperty("--card-width", `${geometry.cardWidth}px`);
  root.setProperty("--card-max-height", `${geometry.cardMaxHeight}px`);
  root.setProperty("--card-radius", `${geometry.cardRadius}px`);
  root.setProperty("--card-padding", `${geometry.cardPadding}px`);
  root.setProperty("--card-top", `${Math.round(800 * geometry.cardTopFraction)}px`);
  root.setProperty("--field-height", `${geometry.fieldHeight}px`);
  root.setProperty("--field-radius", `${geometry.fieldRadius}px`);
  root.setProperty("--row-height", `${geometry.rowHeight}px`);
  root.setProperty("--row-radius", `${geometry.rowRadius}px`);
  root.setProperty("--row-spacing", `${geometry.rowSpacing}px`);
  root.setProperty("--icon-size", `${geometry.iconSize}px`);
  root.setProperty("--title-size", `${geometry.titleSize}px`);
  root.setProperty("--subtitle-size", `${geometry.subtitleSize}px`);
  root.setProperty("--heading-size", `${geometry.headingSize}px`);
  root.setProperty("--query-size", `${geometry.querySize}px`);
  // Structural traits the preset carries beyond the measurements, as data
  // attributes so the stylesheet decides how they look.
  screen.dataset.preset = preset ? preset.name : "gnome";
  screen.dataset.icons = preset && preset.icons ? "on" : "off";
  screen.dataset.fieldRule = preset && preset.fieldRule ? "on" : "off";
  screen.dataset.subtitles = !preset || preset.subtitles ? "on" : "off";
  root.setProperty("--font-stack", fontStack.map((f) => `"${f}"`).join(", ") + ", system-ui, sans-serif");

  screen.classList.toggle("bare", !state.wallpaper);
}

// A stand-in for the icon theme, which a browser has no access to. The real
// launcher resolves the name through the XDG icon theme; here the first
// letter stands in, so row proportions are judged against something of the
// right size rather than against nothing.
function icon(name, title) {
  const element = document.createElement("div");
  element.className = "icon";
  element.textContent = (title || name || "?").slice(0, 1).toUpperCase();
  element.title = `icon: ${name}`;
  return element;
}

function resultRow(row) {
  const element = document.createElement("div");
  element.className = row.selected ? "row selected" : "row";
  element.append(icon(row.icon, row.title));

  const labels = document.createElement("div");
  labels.className = "labels";
  const title = document.createElement("div");
  title.className = "title";
  title.textContent = row.title;
  labels.append(title);
  if (row.subtitle) {
    const subtitle = document.createElement("div");
    subtitle.className = "subtitle";
    subtitle.textContent = row.subtitle;
    labels.append(subtitle);
  }
  element.append(labels);
  return element;
}

function panelElement(panel) {
  const element = document.createElement("div");
  element.className = "panel";

  const filter = document.createElement("input");
  filter.className = "panel-filter";
  filter.placeholder = "Search…";
  filter.setAttribute("aria-label", "Search actions");
  filter.value = panel.filter;
  // Fixture rows were filtered by Rust. This input mirrors the native field's
  // appearance, not its interactions; don't accept edits without reranking.
  filter.readOnly = true;
  element.append(filter);

  for (const row of panel.rows) {
    if (row.kind === "divider") {
      const divider = document.createElement("div");
      divider.className = "divider";
      element.append(divider);
      continue;
    }
    if (row.kind === "header") {
      const header = document.createElement("div");
      header.className = "heading";
      header.textContent = row.title;
      element.append(header);
      continue;
    }
    const item = document.createElement("div");
    item.className = row.selected ? "row selected" : "row";
    const title = document.createElement("div");
    title.className = "title";
    title.textContent = row.title;
    item.append(title);
    if (row.shortcut) {
      const shortcut = document.createElement("div");
      shortcut.className = "shortcut";
      shortcut.textContent = row.shortcut;
      item.append(shortcut);
    }
    element.append(item);
  }
  if (panel.rows.length === 0) {
    const empty = document.createElement("div");
    empty.className = "empty";
    empty.textContent = "No actions";
    element.append(empty);
  }
  return element;
}

function render() {
  applyTokens();
  const current = state.data.states[state.index];
  document.getElementById("description").textContent = current.description;

  const card = document.getElementById("card");
  card.replaceChildren();

  const field = document.createElement("div");
  field.className = "field";
  const glyph = document.createElement("span");
  glyph.className = "glyph";
  glyph.textContent = "⌕";
  field.append(glyph);
  if (current.query) {
    const typed = document.createElement("span");
    typed.textContent = current.query;
    field.append(typed);
  } else {
    const placeholder = document.createElement("span");
    placeholder.className = "placeholder";
    placeholder.textContent = "Search…";
    field.append(placeholder);
  }
  const caret = document.createElement("span");
  caret.className = "caret";
  if (!current.panel) field.append(caret);
  card.append(field);

  const list = document.createElement("div");
  list.className = "list";
  const hasRows = current.sections.some((s) => s.rows.length > 0);
  if (!hasRows) {
    const empty = document.createElement("div");
    empty.className = "empty";
    empty.textContent = current.query ? "No results" : "Type to search";
    list.append(empty);
  } else {
    for (const section of current.sections) {
      if (section.rows.length === 0) continue;
      const heading = document.createElement("div");
      heading.className = "heading";
      heading.textContent = section.heading;
      list.append(heading);
      for (const row of section.rows) list.append(resultRow(row));
    }
  }
  card.append(list);
  list.querySelector(".row.selected")?.scrollIntoView({ block: "nearest" });

  if (current.panel) {
    card.append(panelElement(current.panel));
    card.querySelector(".panel-filter").focus({ preventScroll: true });
  }
  card.dataset.state = current.name;
}

load();
