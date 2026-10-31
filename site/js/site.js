// OASIS_OS shared chrome: top-bar border on scroll + mobile menu.
// Loaded by every page with the shared top bar; no dependencies.
(function () {
  "use strict";

  var topbar = document.getElementById("topbar");
  var toggle = document.getElementById("nav-toggle");
  var nav = document.getElementById("site-nav");

  function onScroll() {
    if (topbar) topbar.classList.toggle("is-scrolled", window.scrollY > 8);
  }
  window.addEventListener("scroll", onScroll, { passive: true });
  onScroll();

  function setMenu(open) {
    if (!toggle || !nav) return;
    nav.classList.toggle("is-open", open);
    toggle.setAttribute("aria-expanded", String(open));
  }
  if (toggle && nav) {
    toggle.addEventListener("click", function () {
      setMenu(toggle.getAttribute("aria-expanded") !== "true");
    });
    nav.addEventListener("click", function (e) {
      if (e.target.closest("a")) setMenu(false);
    });
    document.addEventListener("keydown", function (e) {
      if (e.key === "Escape" && nav.classList.contains("is-open")) {
        setMenu(false);
        toggle.focus();
      }
    });
    // Close the drawer when the viewport grows into the desktop layout,
    // so it isn't left open (and aria-expanded stale) behind the inline nav.
    var desktop = window.matchMedia("(min-width: 1100px)");
    var onChange = function (e) { if (e.matches) setMenu(false); };
    if (desktop.addEventListener) desktop.addEventListener("change", onChange);
    else if (desktop.addListener) desktop.addListener(onChange);
  }
})();
