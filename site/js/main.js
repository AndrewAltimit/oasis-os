// OASIS_OS landing page interactivity

(function () {
  "use strict";

  // -----------------------------------------------------------------------
  // Mobile navigation toggle
  // -----------------------------------------------------------------------

  const navToggle = document.getElementById("nav-toggle");
  const navLinks = document.getElementById("nav-links");

  if (navToggle && navLinks) {
    navToggle.addEventListener("click", () => {
      navLinks.classList.toggle("open");
      navToggle.setAttribute(
        "aria-expanded",
        navLinks.classList.contains("open")
      );
    });

    // Close on link click (mobile)
    navLinks.querySelectorAll("a").forEach((link) => {
      link.addEventListener("click", () => {
        navLinks.classList.remove("open");
        navToggle.setAttribute("aria-expanded", "false");
      });
    });
  }

  // -----------------------------------------------------------------------
  // Smooth scroll for anchor links
  // -----------------------------------------------------------------------

  document.querySelectorAll('a[href^="#"]').forEach((anchor) => {
    anchor.addEventListener("click", (e) => {
      const href = anchor.getAttribute("href");
      if (href === "#" || href === "") return;
      const target = document.querySelector(href);
      if (target) {
        e.preventDefault();
        target.scrollIntoView({ behavior: "smooth", block: "start" });
      }
    });
  });

  // -----------------------------------------------------------------------
  // Skin gallery tabs
  // -----------------------------------------------------------------------

  const skinTabs = document.querySelectorAll(".skin-tab");
  const skinPanels = document.querySelectorAll(".skin-panel");

  skinTabs.forEach((tab) => {
    tab.addEventListener("click", () => {
      const skinId = tab.dataset.skin;

      skinTabs.forEach((t) => t.classList.remove("active"));
      tab.classList.add("active");

      skinPanels.forEach((panel) => {
        panel.classList.toggle("active", panel.id === "skin-" + skinId);
      });
    });
  });

  // -----------------------------------------------------------------------
  // Scroll-triggered fade-in
  // -----------------------------------------------------------------------

  const fadeEls = document.querySelectorAll(".fade-in");

  if (fadeEls.length > 0 && "IntersectionObserver" in window) {
    const observer = new IntersectionObserver(
      (entries) => {
        entries.forEach((entry) => {
          if (entry.isIntersecting) {
            entry.target.classList.add("visible");
            observer.unobserve(entry.target);
          }
        });
      },
      { threshold: 0.1 }
    );
    fadeEls.forEach((el) => observer.observe(el));
  } else {
    // Fallback: show everything immediately
    fadeEls.forEach((el) => el.classList.add("visible"));
  }

  // -----------------------------------------------------------------------
  // Navbar background on scroll
  // -----------------------------------------------------------------------

  const nav = document.querySelector(".site-nav");
  if (nav) {
    window.addEventListener(
      "scroll",
      () => {
        nav.classList.toggle("scrolled", window.scrollY > 40);
      },
      { passive: true }
    );
  }
})();
