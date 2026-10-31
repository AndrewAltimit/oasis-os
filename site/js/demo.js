// OASIS_OS live demo chrome: skin picker, controls card, full-screen
// toggle, boot indicator. The WASM itself is booted by index.js (copied from
// www/index.js), which sets window.__oasisReady once the shell is running,
// or window.__oasisBootError (+ an "oasis:boot-error" event) if it failed.
(function () {
  "use strict";

  // The WASM backend boots "xp" when no ?skin= is given (oasis-backend-wasm
  // OasisWasm::new), so the picker shows the same default.
  var DEFAULT_SKIN = "xp";

  var params = new URLSearchParams(window.location.search);
  var select = document.getElementById("skin-select");
  if (select) {
    var want = params.get("skin") || DEFAULT_SKIN;
    var known = Array.prototype.some.call(select.options, function (o) { return o.value === want; });
    if (known) select.value = want;
    select.addEventListener("change", function () {
      var url = new URL(window.location.href);
      url.searchParams.set("skin", select.value);
      window.location.href = url.toString();
    });
  }
  var sub = document.getElementById("boot-sub");
  if (sub && select && select.selectedIndex >= 0) {
    sub.textContent = "Loading the WebAssembly build · " +
      select.options[select.selectedIndex].text + " skin";
  }

  // Full screen: the stage (#container) so the canvas re-fits to the screen
  // and the web-page overlay index.js attaches there comes along.
  var fsBtn = document.getElementById("fullscreen-btn");
  var stage = document.getElementById("container");
  if (fsBtn && stage && document.fullscreenEnabled && stage.requestFullscreen) {
    fsBtn.hidden = false;
    fsBtn.addEventListener("click", function () {
      if (document.fullscreenElement) document.exitFullscreen();
      else stage.requestFullscreen().catch(function () { /* denied: stay windowed */ });
    });
    document.addEventListener("fullscreenchange", function () {
      var on = !!document.fullscreenElement;
      fsBtn.setAttribute("aria-label", on ? "Exit full screen" : "Full screen");
      fsBtn.classList.toggle("is-on", on);
      var canvas = document.getElementById("oasis");
      if (canvas) canvas.focus();
    });
  }

  // Controls card: dismissible, re-openable, auto-hides after a while.
  var card = document.getElementById("instructions");
  var dismiss = document.getElementById("dismiss-btn");
  var help = document.getElementById("help-btn");
  var reduceMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  var autoHide = null;

  function hideCard(focusCanvas) {
    if (!card || card.hidden) return;
    clearTimeout(autoHide);
    var done = function () {
      card.hidden = true;
      card.classList.remove("is-leaving");
      if (help) help.setAttribute("aria-expanded", "false");
    };
    if (reduceMotion) done();
    else { card.classList.add("is-leaving"); setTimeout(done, 300); }
    if (focusCanvas) {
      var canvas = document.getElementById("oasis");
      if (canvas) canvas.focus();
    }
  }
  function showCard() {
    if (!card) return;
    card.hidden = false;
    if (help) help.setAttribute("aria-expanded", "true");
    if (dismiss) dismiss.focus();
  }
  if (dismiss) dismiss.addEventListener("click", function () { hideCard(true); });
  if (help) {
    help.addEventListener("click", function () {
      if (card && card.hidden) showCard(); else hideCard(true);
    });
  }
  if (card) {
    autoHide = setTimeout(function () { hideCard(false); }, 10000);
    card.addEventListener("keydown", function (e) {
      if (e.key === "Escape") { e.stopPropagation(); hideCard(true); }
    });
  }

  // Boot indicator: hidden once index.js reports the shell is running.
  // If the WASM build is missing (index.js or pkg/ 404 — e.g. site/ served
  // without running scripts/build-wasm.sh) or the browser lacks WebAssembly,
  // the indicator turns into a fallback card instead of spinning forever.
  var boot = document.getElementById("boot");
  var failed = null; // null, or why the demo could not start

  function fail(kind) {
    if (failed && !(failed === "slow" && kind === "error")) return;
    if (window.__oasisReady || !boot) return;
    failed = kind;
    clearTimeout(autoHide);
    if (card) card.hidden = true;
    if (help) help.setAttribute("aria-expanded", "false");
    var brand = document.querySelector(".demo-bar .brand");
    var homeHref = brand ? brand.getAttribute("href") : "../";
    var detail = window.__oasisBootError;
    var why = kind === "error"
      ? "The shell failed to start: <code>" +
        String((detail && detail.message) || detail || "unknown error")
          .replace(/&/g, "&amp;").replace(/</g, "&lt;").slice(0, 200) +
        "</code>. Reloading usually helps; if it keeps happening, the browser console " +
        "has the details."
      : kind === "nowasm"
      ? "This browser does not support WebAssembly, which the demo needs. Any current " +
        "Chrome, Edge, Firefox or Safari will run it."
      : kind === "slow"
        ? "The shell has not started after 30 seconds. A slow connection or a blocked " +
          "download can cause this; reloading usually helps."
        : "The WebAssembly build is not present on this server. The public site builds it " +
          "on deploy; locally, run <code>./scripts/build-wasm.sh</code> and open " +
          "<code>/www/</code> from the repository root.";
    boot.className = "boot is-failed";
    boot.innerHTML =
      '<div class="boot__card">' +
      '<p class="boot__code">' + (kind === "slow" ? "Still booting" : kind === "error"
        ? "Boot failed" : "Demo unavailable") + "</p>" +
      '<p class="boot__why">' + why + "</p>" +
      '<div class="boot__actions">' +
      (kind === "slow" || kind === "error"
        ? '<button type="button" class="btn btn--primary btn--sm" id="boot-reload">Reload</button>'
        : "") +
      '<a class="btn btn--' + (kind === "slow" || kind === "error" ? "ghost" : "primary") + ' btn--sm" href="' +
      homeHref + '">Watch the video tour</a>' +
      '<a class="btn btn--ghost btn--sm" href="https://github.com/AndrewAltimit/oasis-os/blob/main/docs/wasm-backend.md">Build notes</a>' +
      "</div></div>";
    var reload = document.getElementById("boot-reload");
    if (reload) reload.addEventListener("click", function () { window.location.reload(); });
  }

  if (typeof WebAssembly !== "object") fail("nowasm");
  if (window.__oasisBootError) fail("error");
  window.addEventListener("oasis:boot-error", function () { fail("error"); });
  // index.js (and the pkg/ module it imports) fire "error" on the <script>
  // element when they cannot be fetched.
  Array.prototype.forEach.call(document.querySelectorAll('script[type="module"]'), function (s) {
    s.addEventListener("error", function () { fail("missing"); });
  });

  if (boot) {
    var started = Date.now();
    (function poll() {
      if (window.__oasisReady) {
        boot.classList.add("is-done");
        // A late start after the "slow" card still wins.
        if (failed) boot.innerHTML = "";
        return;
      }
      if (failed && failed !== "slow") return; // nothing left to wait for
      var waited = Date.now() - started;
      if (waited > 12000 && !boot.classList.contains("is-slow")) {
        boot.classList.add("is-slow");
        if (sub) sub.textContent = "Still loading — the first visit downloads the whole OS";
      }
      if (waited > 30000) fail("slow");
      setTimeout(poll, 150);
    })();
  }
})();
