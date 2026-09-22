(function(){
  function climb(el, pred) {
    while (el) {
      if (pred(el)) return el;
      el = el.parentNode;
    }
    return null;
  }
  function nearestComment(el) {
    return climb(el, function(n){
      return n.classList && n.classList.contains('comment');
    });
  }
  function nearestMidcol(el) {
    return climb(el, function(n){
      return n.classList && n.classList.contains('midcol');
    });
  }
  function nearestThing(el) {
    return climb(el, function(n){
      return n.classList && n.classList.contains('thing');
    });
  }
  if (typeof globalThis.togglecomment === 'undefined') {
    globalThis.togglecomment = function(el) {
      var c = nearestComment(el);
      if (!c) return false;
      if (c.classList.contains('collapsed')) {
        c.classList.remove('collapsed');
        c.classList.add('noncollapsed');
      } else {
        c.classList.add('collapsed');
        c.classList.remove('noncollapsed');
      }
      return false;
    };
  }
  if (typeof globalThis.hidecomment === 'undefined') {
    globalThis.hidecomment = function(el) {
      var c = nearestComment(el);
      if (c) { c.classList.add('collapsed'); c.classList.remove('noncollapsed'); }
      return false;
    };
  }
  if (typeof globalThis.unhidecomment === 'undefined') {
    globalThis.unhidecomment = function(el) {
      var c = nearestComment(el);
      if (c) { c.classList.remove('collapsed'); c.classList.add('noncollapsed'); }
      return false;
    };
  }
  // Reddit's 'load more comments' can't actually fetch without a JSON
  // bridge, but stubbing it prevents reference errors from onclick.
  if (typeof globalThis.morechildren === 'undefined') {
    globalThis.morechildren = function(){ return false; };
  }
  // Score-update helper: reads the midcol's stored baseline score, adds
  // +1 / -1 / 0 based on which sibling arrow is active, and writes the
  // new total back into `.score`. Also swaps the score's colour class
  // (`likes` orange / `dislikes` blue / `unvoted` gray) so the number
  // changes hue when the user toggles a vote.
  function updateScore(midcol) {
    if (!midcol) return;
    var score = midcol.querySelector('.score');
    if (!score) return;
    // Stash the original score value once, then recompute from it
    // every toggle so the math stays idempotent across re-clicks.
    var base;
    if (midcol.getAttribute('data-base-score')) {
      base = parseInt(midcol.getAttribute('data-base-score'), 10);
    } else {
      var m = ((score.textContent || '').trim()).match(/^(-?\d+)/);
      // Hidden-score posts render as "•" with no numeric prefix;
      // bail out so a toggle doesn't overwrite the bullet with "1".
      if (!m) return;
      base = parseInt(m[1], 10);
      midcol.setAttribute('data-base-score', String(base));
      // Stash original suffix too — "412 points" vs plain "412".
      var suf = ((score.textContent || '').trim()).replace(/^-?\d+/, '');
      midcol.setAttribute('data-score-suffix', suf);
    }
    var suffix = midcol.getAttribute('data-score-suffix') || '';
    var arrows = midcol.querySelectorAll('.arrow');
    var delta = 0;
    var upActive = false, downActive = false;
    for (var i = 0; i < arrows.length; i++) {
      if (arrows[i].classList.contains('upmod')) { delta += 1; upActive = true; }
      else if (arrows[i].classList.contains('downmod')) { delta -= 1; downActive = true; }
    }
    score.textContent = (base + delta) + suffix;
    score.classList.remove('likes');
    score.classList.remove('dislikes');
    score.classList.remove('unvoted');
    if (upActive) score.classList.add('likes');
    else if (downActive) score.classList.add('dislikes');
    else score.classList.add('unvoted');
  }
  // Vote arrows: toggle `upmod`/`downmod` on the clicked arrow, clear
  // the opposite arrow (reddit treats up+down as mutually exclusive),
  // and recompute the displayed score. Clicking the same arrow twice
  // returns to the unvoted state.
  if (typeof globalThis.togglevote === 'undefined') {
    globalThis.togglevote = function(el, dir) {
      if (!el || !el.classList) return false;
      var on = dir > 0 ? 'upmod' : 'downmod';
      var off = dir > 0 ? 'up' : 'down';
      var wasActive = el.classList.contains(on);
      if (wasActive) {
        el.classList.remove(on);
        el.classList.add(off);
      } else {
        el.classList.add(on);
        el.classList.remove(off);
        // Deactivate opposite arrow in the same midcol.
        var midcol = nearestMidcol(el);
        if (midcol) {
          var opposite = dir > 0 ? 'downmod' : 'upmod';
          var oppOff = dir > 0 ? 'down' : 'up';
          var sibs = midcol.querySelectorAll('.arrow');
          for (var i = 0; i < sibs.length; i++) {
            var s = sibs[i];
            if (s !== el && s.classList.contains(opposite)) {
              s.classList.remove(opposite);
              s.classList.add(oppOff);
            }
          }
        }
      }
      updateScore(nearestMidcol(el));
      return false;
    };
  }
  // Listing pages don't wire an onclick on each arrow — reddit's real
  // JS uses event delegation on the content block. We scan once at
  // DOMContentLoaded and attach a fallback listener to every `.arrow`
  // that doesn't already have an inline onclick. A DOM marker attribute
  // keeps re-navigations idempotent.
  // Scope to `.midcol .arrow` so non-Reddit pages that happen to use
  // `.arrow` for nav chevrons / dropdowns don't have reddit vote
  // semantics injected (the togglevote handler would otherwise add
  // `upmod`/`downmod` classes to unrelated elements).
  function wireListingArrows() {
    var arrows = document.querySelectorAll('.midcol .arrow');
    for (var i = 0; i < arrows.length; i++) {
      var a = arrows[i];
      if (a.getAttribute('onclick')) continue;
      if (a.getAttribute('data-oasis-vote-wired')) continue;
      a.setAttribute('data-oasis-vote-wired', '1');
      (function(arrow){
        arrow.addEventListener('click', function(ev) {
          var dir = (arrow.classList.contains('up') ||
                     arrow.classList.contains('upmod')) ? 1 : -1;
          globalThis.togglevote(arrow, dir);
          if (ev && ev.preventDefault) ev.preventDefault();
        });
      })(a);
    }
  }
  wireListingArrows();
  // Gate reddit-specific DOM rewrites (`wireHideButtons`, `wireTabmenu`)
  // to contexts where reddit semantics actually apply. Without this,
  // clicking `.tabmenu li a` on MediaWiki / phpBB / any other CMS that
  // uses the `.tabmenu` class (or a page with `.thing .buttons a`) would
  // silently mutate their `.selected` state and break their own JS. For
  // live http(s) pages we require `reddit.com` in the URL; offline
  // contexts (vfs:// fixtures, file://, etc.) are allowed through so
  // reddit fixtures continue to work in tests.
  function isRedditContext() {
    var here = '';
    try { here = String(__oasis_location() || ''); } catch (e) { return false; }
    if (here.indexOf('http://') === 0 || here.indexOf('https://') === 0) {
      return here.indexOf('reddit.com') !== -1;
    }
    return true;
  }
  // Hide button: reddit marks hidden posts with a `.hidden` class on
  // the outer `.thing` and the stylesheet renders them as `display:none`.
  // We attach a click handler to every `.thing .buttons a` whose text is
  // literally "hide" so the link toggles the thing from the layout.
  function wireHideButtons() {
    if (!isRedditContext()) return;
    var anchors = document.querySelectorAll('.thing .buttons a');
    for (var i = 0; i < anchors.length; i++) {
      var a = anchors[i];
      if (a.getAttribute('onclick')) continue;
      if (a.getAttribute('data-oasis-hide-wired')) continue;
      var text = (a.textContent || '').trim().toLowerCase();
      if (text !== 'hide' && text !== 'unhide') continue;
      a.setAttribute('data-oasis-hide-wired', '1');
      (function(anchor){
        anchor.addEventListener('click', function(ev) {
          var thing = nearestThing(anchor);
          if (thing) {
            if (thing.classList.contains('hidden')) {
              thing.classList.remove('hidden');
            } else {
              thing.classList.add('hidden');
            }
          }
          if (ev && ev.preventDefault) ev.preventDefault();
        });
      })(a);
    }
  }
  wireHideButtons();
  // Sort tabs inside `.tabmenu`: clicking an entry moves the
  // `.selected` class to the parent `<li>` so the visual state at
  // least tracks the user's click. On a VFS-served fixture there's
  // nowhere to navigate to, so we also suppress the link; on a live
  // http(s) page we let the click fall through to the engine's
  // navigation (which will load the new URL and re-render the tab
  // row with the correct `.selected` entry).
  function wireTabmenu() {
    if (!isRedditContext()) return;
    var tabs = document.querySelectorAll('.tabmenu li a');
    for (var i = 0; i < tabs.length; i++) {
      var a = tabs[i];
      if (a.getAttribute('data-oasis-tab-wired')) continue;
      a.setAttribute('data-oasis-tab-wired', '1');
      (function(anchor){
        anchor.addEventListener('click', function(ev) {
          var li = anchor.parentNode;
          if (li && li.parentNode) {
            var siblings = li.parentNode.querySelectorAll('li');
            for (var j = 0; j < siblings.length; j++) {
              siblings[j].classList.remove('selected');
            }
            li.classList.add('selected');
          }
          // Only suppress navigation in offline/VFS-like contexts
          // where the href can't resolve. Live http(s) pages should
          // follow the link normally. Fail-closed on purpose: if the
          // __oasis_location binding is unavailable (throws), `here`
          // stays '' and we suppress navigation — safer than letting
          // an unresolvable href load a blank page.
          var here = '';
          try { here = String(__oasis_location() || ''); } catch (e) {}
          if (here.indexOf('http://') !== 0 && here.indexOf('https://') !== 0
              && ev && ev.preventDefault) {
            ev.preventDefault();
          }
        });
      })(a);
    }
  }
  wireTabmenu();
})();
