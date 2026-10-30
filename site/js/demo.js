// OASIS_OS live demo chrome: skin picker, controls card, boot indicator.
// The WASM itself is booted by index.js (copied from www/index.js), which
// sets window.__oasisReady once the shell is running.
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
  var boot = document.getElementById("boot");
  if (boot) {
    var started = Date.now();
    (function poll() {
      if (window.__oasisReady) { boot.classList.add("is-done"); return; }
      if (Date.now() - started > 15000) boot.classList.add("is-slow");
      setTimeout(poll, 150);
    })();
  }
})();
