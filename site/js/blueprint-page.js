// OASIS_OS blueprint page: loads the 3D viewer only when it is needed, and
// owns the small bits of page chrome the engine does not (legend toggle).
//
// three.js + its glTF loader are ~700 KB; the gallery (no ?diagram=) needs
// neither, so they are only fetched for a diagram. The scripts are inserted
// in order (async = false) while the page is still parsing, so they run
// before the window "load" event that js/blueprint-engine.js initialises on.
(function () {
  "use strict";

  var wantsDiagram = /[?&]diagram=/.test(window.location.search);
  var scripts = [
    "js/blueprint-engine.js",
    "js/blueprints/psp-relay.js",
    "js/blueprints/psp-usb-adapter.js"
  ];
  if (wantsDiagram) scripts.unshift("js/three.min.js", "js/GLTFLoader.js");
  scripts.forEach(function (src) {
    var el = document.createElement("script");
    el.src = src;
    el.async = false;
    document.body.appendChild(el);
  });

  // Gallery: drop the "Loading 3D engine" cover at once instead of at "load".
  if (!wantsDiagram) {
    document.body.classList.add("is-gallery");
    var loading = document.getElementById("loading");
    if (loading) loading.style.display = "none";
  }

  // Small screens: the legend starts folded behind a toggle in the dock.
  var toggle = document.getElementById("legend-toggle");
  var legend = document.getElementById("legend");
  if (toggle && legend) {
    toggle.hidden = false;
    toggle.addEventListener("click", function () {
      var open = toggle.getAttribute("aria-expanded") !== "true";
      toggle.setAttribute("aria-expanded", String(open));
      document.body.classList.toggle("legend-open", open);
    });
  }
})();
