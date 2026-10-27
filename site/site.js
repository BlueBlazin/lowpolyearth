// Progressive enhancement only: every photo is a plain link to its JPG and
// every page reads top to bottom without this script.
(() => {
  "use strict";
  const motion = matchMedia("(prefers-reduced-motion: reduce)");

  // Lightbox over the visible [data-photo] links, in page order.
  const dialog = document.querySelector("#viewer");
  if (dialog) {
    const image = dialog.querySelector("img");
    const status = dialog.querySelector(".viewer-status");
    const title = dialog.querySelector("#viewer-title");
    const meta = dialog.querySelector(".viewer-meta");
    const links = () =>
      [...document.querySelectorAll("[data-photo]")].filter(
        (link) => !link.closest("[hidden]"),
      );
    let current = null;
    const show = (link) => {
      current = link;
      const thumbnail = link.querySelector("img");
      image.hidden = true;
      status.textContent = "Loading";
      image.sizes = "100vw";
      image.srcset = thumbnail.srcset;
      image.src = thumbnail.currentSrc || thumbnail.src;
      image.alt = thumbnail.alt;
      title.textContent = link.dataset.title;
      meta.textContent = link.dataset.meta || "";
    };
    const step = (direction) => {
      const list = links();
      const index = list.indexOf(current);
      show(list[(index + direction + list.length) % list.length]);
    };
    image.addEventListener("load", () => {
      image.hidden = false;
      status.textContent = "";
    });
    image.addEventListener("error", () => {
      status.textContent = "The photograph did not load.";
    });
    document.addEventListener("click", (event) => {
      const link = event.target.closest("[data-photo]");
      if (!link || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey)
        return;
      event.preventDefault();
      show(link);
      dialog.showModal();
    });
    dialog.querySelector("[data-close]").addEventListener("click", () => dialog.close());
    dialog.querySelectorAll("[data-step]").forEach((button) =>
      button.addEventListener("click", () => step(Number(button.dataset.step))),
    );
    dialog.addEventListener("keydown", (event) => {
      if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
        event.preventDefault();
        step(event.key === "ArrowRight" ? 1 : -1);
      }
    });
    dialog.addEventListener("close", () => current?.focus());
  }

  // Muted loop that runs only while visible; reduced motion leaves it paused on its poster.
  const film = document.querySelector(".film video");
  const toggle = document.querySelector("[data-film-toggle]");
  if (film && toggle) {
    let wanted = !motion.matches;
    let inView = false;
    const sync = () => {
      toggle.textContent = film.paused ? "Play" : "Pause";
    };
    film.addEventListener("play", sync);
    film.addEventListener("pause", sync);
    toggle.hidden = false;
    sync();
    toggle.addEventListener("click", () => {
      wanted = film.paused;
      if (wanted) film.play().catch(() => {});
      else film.pause();
    });
    if ("IntersectionObserver" in window) {
      new IntersectionObserver((entries) => {
        inView = entries[0].isIntersecting;
        if (inView && wanted) film.play().catch(() => {});
        else film.pause();
      }, { threshold: 0.25 }).observe(film);
    } else {
      inView = true;
      if (wanted) film.play().catch(() => {});
    }
    document.addEventListener("visibilitychange", () => {
      if (document.hidden) film.pause();
      else if (inView && wanted) film.play().catch(() => {});
    });
  }

  // Gallery filters.
  const filters = [...document.querySelectorAll("[data-filter]")];
  const count = document.querySelector("#photo-count");
  filters.forEach((button) =>
    button.addEventListener("click", () => {
      const value = button.dataset.filter;
      filters.forEach((other) =>
        other.setAttribute("aria-pressed", String(other === button)),
      );
      let shown = 0;
      document.querySelectorAll("[data-category]").forEach((figure) => {
        figure.hidden = value !== "all" && figure.dataset.category !== value;
        if (!figure.hidden) shown += 1;
      });
      if (count) count.textContent = `${shown} photographs`;
    }),
  );
  document.querySelectorAll("[data-js-only]").forEach((element) => {
    element.hidden = false;
  });

  // FAQ answers ease open and closed; without script or with reduced motion
  // the native <details> toggle stays instant.
  document.querySelectorAll(".faq-list details").forEach((details) => {
    const summary = details.querySelector("summary");
    let animation = null;
    let closing = false;
    summary.addEventListener("click", (event) => {
      if (motion.matches || !details.animate) return;
      event.preventDefault();
      // Reversing mid-animation starts from the current height.
      const start = details.offsetHeight;
      const opening = animation ? closing : !details.open;
      animation?.cancel();
      details.open = true;
      const border = details.offsetHeight - details.clientHeight;
      const end = opening ? details.offsetHeight : summary.offsetHeight + border;
      closing = !opening;
      details.style.overflow = "hidden";
      // Holding the last frame until `open` is set avoids a one-frame flash
      // of the full answer when closing.
      const running = details.animate(
        { height: [`${start}px`, `${end}px`] },
        { duration: 240, easing: "cubic-bezier(0.2, 0.6, 0.1, 1)", fill: "forwards" },
      );
      running.onfinish = () => {
        details.open = opening;
        details.style.overflow = "";
        running.cancel();
        animation = null;
      };
      animation = running;
    });
  });

  // Scroll reveal, skipped entirely for reduced motion.
  if (!motion.matches && "IntersectionObserver" in window) {
    const observer = new IntersectionObserver(
      (entries) =>
        entries.forEach((entry) => {
          if (entry.isIntersecting) {
            entry.target.classList.remove("is-pending");
            observer.unobserve(entry.target);
          }
        }),
      { threshold: 0.1 },
    );
    document.querySelectorAll(".reveal").forEach((element) => {
      if (element.getBoundingClientRect().top > innerHeight)
        element.classList.add("is-pending");
      observer.observe(element);
    });
  }
})();
