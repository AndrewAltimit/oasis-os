// OASIS_OS developer's journal. Progressive enhancement only: every article
// reads fine without it.
//   Articles   reading progress, table of contents (sticky on wide screens,
//              a disclosure on narrow ones) with scroll-spy, code windows
//              (language label, light syntax colours, copy button), a
//              lightbox for figures, and Mermaid diagrams (fetched from a
//              CDN only on pages that contain one).
//   Index      topic filter chips and a newest/oldest sort over the entry cards.
(function () {
  "use strict";

  var $$ = function (sel, root) {
    return Array.prototype.slice.call((root || document).querySelectorAll(sel));
  };
  var reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;

  // ------------------------------------------------------------------
  // Reading progress (a transform, so it never triggers layout).
  // ------------------------------------------------------------------
  var bar = document.getElementById("progress-bar");
  var prose = document.querySelector(".prose");
  if (bar && prose) {
    var ticking = false;
    var update = function () {
      ticking = false;
      var r = prose.getBoundingClientRect();
      var total = r.height - window.innerHeight * 0.6;
      var p = total > 0 ? Math.min(1, Math.max(0, -r.top / total)) : 1;
      bar.style.transform = "scaleX(" + p.toFixed(4) + ")";
    };
    var request = function () {
      if (!ticking) { ticking = true; window.requestAnimationFrame(update); }
    };
    window.addEventListener("scroll", request, { passive: true });
    window.addEventListener("resize", request);
    update();
  }

  // ------------------------------------------------------------------
  // Table of contents: disclosure on narrow screens + scroll-spy.
  // ------------------------------------------------------------------
  var toc = document.querySelector(".toc");
  if (toc) {
    var title = toc.querySelector(".toc__title");
    var list = toc.querySelector("ol");
    if (title && list) {
      // A toggle that CSS shows (in place of the title) only below the wide
      // layout, where the list starts collapsed.
      list.id = list.id || "toc-list";
      var btn = document.createElement("button");
      btn.type = "button";
      btn.className = "toc__toggle";
      btn.setAttribute("aria-expanded", "false");
      btn.setAttribute("aria-controls", list.id);
      var first = list.querySelector("a");
      btn.innerHTML = '<span class="toc__label">' + title.textContent + "</span>" +
        '<span class="toc__now"></span>';
      btn.querySelector(".toc__now").textContent = first ? first.textContent : "";
      toc.insertBefore(btn, title);
      toc.classList.add("is-enhanced");
      btn.addEventListener("click", function () {
        var open = btn.getAttribute("aria-expanded") !== "true";
        btn.setAttribute("aria-expanded", String(open));
        toc.classList.toggle("is-open", open);
      });
      toc.addEventListener("click", function (e) {
        if (e.target.closest("ol a")) {
          btn.setAttribute("aria-expanded", "false");
          toc.classList.remove("is-open");
        }
      });
    }
  }

  var links = $$(".toc ol a");
  if (links.length) {
    var byId = {};
    links.forEach(function (a) { byId[decodeURIComponent(a.hash.slice(1))] = a; });
    var heads = $$(".prose h2[id]").filter(function (h) { return byId[h.id]; });
    var current = null;
    var setCurrent = function (id) {
      if (id === current) return;
      current = id;
      links.forEach(function (a) {
        var on = a === byId[id];
        a.classList.toggle("is-current", on);
        if (on) a.setAttribute("aria-current", "location");
        else a.removeAttribute("aria-current");
      });
      var active = byId[id];
      var now = document.querySelector(".toc__now");
      if (now && active) now.textContent = active.textContent;
      // Keep the active entry visible inside a scrolling sticky TOC.
      var ol = active && active.closest("ol");
      if (ol && ol.scrollHeight > ol.clientHeight) {
        var top = active.offsetTop - ol.clientHeight / 2;
        ol.scrollTo({ top: top, behavior: reduceMotion ? "auto" : "smooth" });
      }
    };
    var spy = function () {
      // The current section is the last heading above the top third of the viewport.
      var line = window.innerHeight * 0.33;
      var pick = heads[0];
      heads.forEach(function (h) { if (h.getBoundingClientRect().top < line) pick = h; });
      if (pick) setCurrent(pick.id);
    };
    // Scroll events (rAF-throttled) rather than an IntersectionObserver alone:
    // a jump (TOC click, find-in-page, restored scroll) can skip past headings
    // without any of them crossing the observer's edge.
    var spyQueued = false;
    window.addEventListener("scroll", function () {
      if (spyQueued) return;
      spyQueued = true;
      window.requestAnimationFrame(function () { spyQueued = false; spy(); });
    }, { passive: true });
    window.addEventListener("resize", spy);
    spy();
  }

  // ------------------------------------------------------------------
  // Code windows: wrap each prose <pre> in the site's .codeblock chrome
  // with a language label, light highlighting and a copy button.
  // ASCII diagrams (.diagram) and Mermaid sources are left alone.
  // ------------------------------------------------------------------
  var LANGS = {
    rust: {
      label: "rust",
      kw: "as async await break const continue crate dyn else enum extern false fn for if impl " +
        "in let loop match mod move mut pub ref return self Self static struct super trait " +
        "true type unsafe use where while",
      line: "//"
    },
    c: {
      label: "c",
      kw: "auto break case char const continue default do double else enum extern float for " +
        "goto if int long register return short signed sizeof static struct switch typedef " +
        "union unsigned void volatile while u8 u16 u32 u64 s32 uint8_t uint16_t uint32_t " +
        "int32_t size_t bool true false NULL",
      line: "//"
    },
    js: {
      label: "javascript",
      kw: "async await break case catch class const continue default delete do else export " +
        "false finally for function if import in instanceof let new null return switch this " +
        "throw true try typeof undefined var void while",
      line: "//"
    },
    shell: {
      label: "shell",
      kw: "cd export if then fi for do done in echo sudo",
      line: "#"
    },
    toml: { label: "toml", kw: "true false", line: "#" }
  };

  function detect(text) {
    if (/^\s*\[[\w.-]+\]\s*$/m.test(text) && /^\s*[\w-]+\s*=/m.test(text) &&
        !/\bfn\b|;\s*$/m.test(text)) return "toml";
    if (/\b(fn|impl|let mut|pub (fn|struct|trait|enum)|unsafe \{|use [\w:]+;|#\[|&mut |::)/.test(text)) {
      return "rust";
    }
    if (/#include|\b(void|int|u32|uint32_t)\s+\**\w+\s*\(/.test(text)) return "c";
    if (/\b(function|const|let|var)\s+\w+|=>|console\.|document\./.test(text)) return "js";
    if (/^\s*(\$ |cargo |docker |python3? |cd |git |\.\/|scripts\/|ffmpeg |curl |echo |nc )/m.test(text)) {
      return "shell";
    }
    return null;
  }

  function esc(s) {
    return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  }

  // A deliberately small tokenizer: comments, strings, numbers, keywords,
  // macros and CamelCase types. Good enough to give long listings texture.
  function highlight(text, lang) {
    var spec = LANGS[lang];
    var kw = {};
    spec.kw.split(" ").forEach(function (k) { kw[k] = true; });
    var lineC = spec.line === "#" ? "#[^\\n]*" : "\\/\\/[^\\n]*";
    var parts = [
      "(" + lineC + "|\\/\\*[\\s\\S]*?\\*\\/)",          // 1 comment
      "(\"(?:[^\"\\\\\\n]|\\\\.)*\"|'(?:[^'\\\\\\n]|\\\\.)'|`[^`]*`)", // 2 string / char
      "(\\b0x[0-9A-Fa-f_]+\\b|\\b\\d[\\d_]*(?:\\.\\d+)?(?:[a-z]\\w*)?\\b)", // 3 number
      "([A-Za-z_]\\w*!)",                                 // 4 macro
      "([A-Za-z_]\\w*)"                                   // 5 identifier
    ];
    if (lang === "shell") parts[0] = "((?:^|\\s)#[^\\n]*)";
    var re = new RegExp(parts.join("|"), "gm");
    var out = "";
    var last = 0;
    var m;
    while ((m = re.exec(text)) !== null) {
      out += esc(text.slice(last, m.index));
      var tok = m[0];
      var cls = null;
      if (m[1]) cls = "tc";
      else if (m[2]) cls = "ts";
      else if (m[3]) cls = "tn";
      else if (m[4]) cls = lang === "rust" ? "tm" : null;
      else if (m[5]) {
        if (kw[tok]) cls = "tk";
        else if (/^[A-Z][a-z0-9]+[A-Za-z0-9]*$/.test(tok) && lang !== "shell") cls = "tt";
        else if (text.charAt(re.lastIndex) === "(" && lang !== "shell") cls = "tf";
      }
      out += cls ? '<span class="' + cls + '">' + esc(tok) + "</span>" : esc(tok);
      last = re.lastIndex;
    }
    return out + esc(text.slice(last));
  }

  function copyText(text) {
    if (navigator.clipboard && window.isSecureContext) {
      return navigator.clipboard.writeText(text);
    }
    return new Promise(function (resolve, reject) {
      var ta = document.createElement("textarea");
      ta.value = text;
      ta.setAttribute("readonly", "");
      ta.style.position = "fixed";
      ta.style.opacity = "0";
      document.body.appendChild(ta);
      ta.select();
      var ok = false;
      try { ok = document.execCommand("copy"); } catch (e) { ok = false; }
      ta.remove();
      if (ok) resolve(); else reject(new Error("copy failed"));
    });
  }

  var COPY_ICON = '<svg aria-hidden="true" viewBox="0 0 16 16" width="14" height="14">' +
    '<rect x="5" y="5" width="8.5" height="8.5" rx="1.5" fill="none" stroke="currentColor" ' +
    'stroke-width="1.4"/><path d="M10.5 3V2.5A1.5 1.5 0 0 0 9 1H3.5A1.5 1.5 0 0 0 2 2.5V8a1.5 ' +
    '1.5 0 0 0 1.5 1.5H4" fill="none" stroke="currentColor" stroke-width="1.4"/></svg>';

  $$(".prose pre").forEach(function (pre) {
    if (pre.classList.contains("mermaid") || pre.closest(".diagram, .codeblock")) return;
    var code = pre.querySelector("code") || pre;
    var text = code.textContent.replace(/\n$/, "");
    var lang = detect(text);
    // Only colour plain listings; hand-marked-up ones keep their spans.
    if (lang && !code.firstElementChild) code.innerHTML = highlight(text, lang);

    var win = document.createElement("div");
    win.className = "codeblock";
    var barEl = document.createElement("div");
    barEl.className = "codeblock__bar";
    barEl.innerHTML = '<span class="codeblock__lights" aria-hidden="true"><i></i><i></i><i></i></span>' +
      '<span class="codeblock__lang">' + (lang ? LANGS[lang].label : "text") + "</span>";
    var btn = document.createElement("button");
    btn.type = "button";
    btn.className = "codeblock__copy";
    btn.innerHTML = COPY_ICON + "<span>Copy</span>";
    btn.setAttribute("aria-label", "Copy code to clipboard");
    var label = btn.querySelector("span");
    var reset = null;
    btn.addEventListener("click", function () {
      copyText(text).then(function () {
        label.textContent = "Copied";
        btn.classList.add("is-done");
      }, function () {
        label.textContent = "Press Ctrl+C";
      }).then(function () {
        clearTimeout(reset);
        reset = setTimeout(function () {
          label.textContent = "Copy";
          btn.classList.remove("is-done");
        }, 1800);
      });
    });
    barEl.appendChild(btn);
    pre.parentNode.insertBefore(win, pre);
    win.appendChild(barEl);
    win.appendChild(pre);
    pre.setAttribute("tabindex", "0"); // scrollable region reachable by keyboard
  });

  // ------------------------------------------------------------------
  // Figure lightbox: click an inline figure image to view it full size.
  // Pixel-art captures (img.pixel) are scaled by whole multiples and kept
  // crisp with image-rendering: pixelated.
  // ------------------------------------------------------------------
  var figImgs = $$(".prose figure > img");
  if (figImgs.length && typeof HTMLDialogElement === "function") {
    var dlg = document.createElement("dialog");
    dlg.className = "lightbox";
    dlg.setAttribute("aria-label", "Image viewer");
    dlg.innerHTML = '<figure class="lightbox__fig"><img alt=""><figcaption></figcaption></figure>' +
      '<button type="button" class="lightbox__close" aria-label="Close image viewer">' +
      '<svg aria-hidden="true" viewBox="0 0 16 16" width="16" height="16"><path d="M3.5 3.5l9 9M12.5 ' +
      '3.5l-9 9" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/></svg></button>';
    document.body.appendChild(dlg);
    var big = dlg.querySelector("img");
    var cap = dlg.querySelector("figcaption");
    var opener = null;

    var fit = function () {
      if (!big.classList.contains("pixel")) { big.style.width = ""; return; }
      var w = +big.getAttribute("data-w") || big.naturalWidth;
      var h = +big.getAttribute("data-h") || big.naturalHeight;
      if (!w || !h) return;
      var room = Math.min((window.innerWidth - 48) / w, (window.innerHeight - 160) / h);
      var scale = room >= 1 ? Math.floor(room) : room;
      big.style.width = Math.round(w * scale) + "px";
    };

    var open = function (img) {
      opener = img;
      big.src = img.currentSrc || img.src;
      big.alt = img.alt;
      big.className = img.classList.contains("pixel") ? "pixel" : "";
      big.setAttribute("data-w", img.getAttribute("width") || "");
      big.setAttribute("data-h", img.getAttribute("height") || "");
      var fc = img.closest("figure").querySelector("figcaption");
      cap.innerHTML = fc ? fc.innerHTML : "";
      cap.hidden = !fc;
      fit();
      dlg.showModal();
      document.documentElement.classList.add("has-lightbox");
    };

    figImgs.forEach(function (img) {
      img.classList.add("is-zoomable");
      img.setAttribute("tabindex", "0");
      img.setAttribute("role", "button");
      img.setAttribute("aria-label", "Enlarge image: " + img.alt);
      img.addEventListener("click", function () { open(img); });
      img.addEventListener("keydown", function (e) {
        if (e.key === "Enter" || e.key === " ") { e.preventDefault(); open(img); }
      });
    });
    big.addEventListener("load", fit);
    window.addEventListener("resize", function () { if (dlg.open) fit(); });
    dlg.querySelector(".lightbox__close").addEventListener("click", function () { dlg.close(); });
    // A click on the backdrop (the dialog box itself, not its content) closes it.
    dlg.addEventListener("click", function (e) { if (e.target === dlg) dlg.close(); });
    dlg.addEventListener("close", function () {
      document.documentElement.classList.remove("has-lightbox");
      if (opener) opener.focus();
    });
  }

  // ------------------------------------------------------------------
  // Journal index: filter entries by topic.
  // ------------------------------------------------------------------
  var filters = document.getElementById("jfilters");
  var entries = $$(".jlist > li[data-tags]");
  if (filters && entries.length) {
    var count = document.getElementById("jcount");
    filters.hidden = false;
    filters.addEventListener("click", function (e) {
      var chip = e.target.closest(".chip");
      if (!chip) return;
      var tag = chip.getAttribute("data-filter");
      $$(".chip", filters).forEach(function (c) {
        var on = c === chip;
        c.classList.toggle("is-active", on);
        c.setAttribute("aria-pressed", String(on));
      });
      var shown = 0;
      entries.forEach(function (li) {
        var match = tag === "all" || (" " + li.getAttribute("data-tags") + " ")
          .indexOf(" " + tag + " ") !== -1;
        li.hidden = !match;
        if (match) shown += 1;
      });
      if (count) {
        count.textContent = tag === "all"
          ? "Showing all " + shown + " entries"
          : "Showing " + shown + " of " + entries.length + " entries tagged “" +
            chip.firstChild.textContent.trim() + "”";
      }
    });

    // Sort toggle: newest first (the markup order, latest entry featured)
    // or oldest first (read the series in order, no featured card).
    var sort = document.getElementById("jsort");
    var listEl = entries[0].parentNode;
    var featured = listEl.querySelector(".jentry--featured");
    if (sort) {
      sort.addEventListener("click", function () {
        var oldest = !sort.classList.contains("is-oldest");
        sort.classList.toggle("is-oldest", oldest);
        sort.querySelector("span").textContent = oldest ? "Oldest first" : "Newest first";
        sort.setAttribute("aria-label", oldest
          ? "Sort order: oldest first. Switch to newest first"
          : "Sort order: newest first. Switch to oldest first");
        $$(".jlist > li", document).reverse().forEach(function (li) { listEl.appendChild(li); });
        if (featured) featured.classList.toggle("jentry--featured", !oldest);
      });
    }
  }

  // ------------------------------------------------------------------
  // Mermaid: only fetched when the page contains a diagram.
  // ------------------------------------------------------------------
  if (document.querySelector("pre.mermaid")) {
    import("https://cdn.jsdelivr.net/npm/mermaid@11/dist/mermaid.esm.min.mjs")
      .then(function (m) {
        var mermaid = m.default;
        mermaid.initialize({
          startOnLoad: false,
          theme: "base",
          fontFamily: "JetBrains Mono, ui-monospace, monospace",
          themeVariables: {
            darkMode: true,
            background: "#10131b",
            primaryColor: "#151924",
            primaryTextColor: "#eceef4",
            primaryBorderColor: "#2e3449",
            lineColor: "#7d8497",
            secondaryColor: "#151924",
            tertiaryColor: "#10131b",
            fontSize: "14px"
          }
        });
        return mermaid.run({ querySelector: "pre.mermaid" });
      })
      .catch(function (err) {
        // Offline or CDN blocked: the diagram source stays readable as text.
        if (window.console) console.warn("Mermaid unavailable:", err && err.message);
      });
  }
})();
