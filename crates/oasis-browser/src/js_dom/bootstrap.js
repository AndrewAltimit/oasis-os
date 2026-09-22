(function() {
  "use strict";

  function Element(nid) {
    this.__oasis_node_id = nid;
  }

  // Helper to get-or-create an Element wrapper by nid.
  function __get_el(nid) {
    return nid >= 0 ? new Element(nid) : null;
  }

  Object.defineProperties(Element.prototype, {
    tagName: {
      get: function() {
        return __oasis_tagname(this.__oasis_node_id);
      },
      enumerable: true
    },
    id: {
      get: function() {
        return __oasis_getattr(this.__oasis_node_id, "id") || "";
      },
      set: function(v) {
        if (v) __oasis_setattr(this.__oasis_node_id, "id", v);
        else __oasis_rmattr(this.__oasis_node_id, "id");
      },
      enumerable: true
    },
    textContent: {
      get: function() {
        return __oasis_text(this.__oasis_node_id);
      },
      set: function(v) {
        __oasis_settext(this.__oasis_node_id, String(v));
      },
      enumerable: true
    },
    children: {
      get: function() {
        var ids = __oasis_children(this.__oasis_node_id);
        var result = [];
        for (var i = 0; i < ids.length; i++)
          result.push(new Element(ids[i]));
        return result;
      },
      enumerable: true
    },
    parentElement: {
      get: function() {
        var pid = __oasis_parent(this.__oasis_node_id);
        return pid >= 0 ? new Element(pid) : null;
      },
      enumerable: true
    },
    parentNode: {
      get: function() {
        var pid = __oasis_parent(this.__oasis_node_id);
        return pid >= 0 ? new Element(pid) : null;
      },
      enumerable: true
    },
    firstChild: {
      get: function() {
        var ids = __oasis_children(this.__oasis_node_id);
        return ids.length > 0 ? new Element(ids[0]) : null;
      },
      enumerable: true
    },
    lastChild: {
      get: function() {
        var ids = __oasis_children(this.__oasis_node_id);
        return ids.length > 0 ? new Element(ids[ids.length - 1]) : null;
      },
      enumerable: true
    },
    childNodes: {
      get: function() {
        var ids = __oasis_children(this.__oasis_node_id);
        var result = [];
        for (var i = 0; i < ids.length; i++)
          result.push(new Element(ids[i]));
        return result;
      },
      enumerable: true
    },
    nextSibling: {
      get: function() {
        var pid = __oasis_parent(this.__oasis_node_id);
        if (pid < 0) return null;
        var siblings = __oasis_children(pid);
        for (var i = 0; i < siblings.length - 1; i++) {
          if (siblings[i] === this.__oasis_node_id) return new Element(siblings[i + 1]);
        }
        return null;
      },
      enumerable: true
    },
    previousSibling: {
      get: function() {
        var pid = __oasis_parent(this.__oasis_node_id);
        if (pid < 0) return null;
        var siblings = __oasis_children(pid);
        for (var i = 1; i < siblings.length; i++) {
          if (siblings[i] === this.__oasis_node_id) return new Element(siblings[i - 1]);
        }
        return null;
      },
      enumerable: true
    },
    innerHTML: {
      get: function() {
        return __oasis_inner_html(this.__oasis_node_id);
      },
      set: function(v) {
        __oasis_set_inner_html(this.__oasis_node_id, String(v));
      },
      enumerable: true
    },
    classList: {
      get: function() {
        var self = this;
        return {
          add: function(c) {
            __oasis_classlist_op(self.__oasis_node_id, 'add', c);
          },
          remove: function(c) {
            __oasis_classlist_op(
              self.__oasis_node_id, 'remove', c
            );
          },
          toggle: function(c) {
            return __oasis_classlist_op(
              self.__oasis_node_id, 'toggle', c
            );
          },
          contains: function(c) {
            return __oasis_classlist_op(
              self.__oasis_node_id, 'contains', c
            );
          }
        };
      },
      enumerable: true
    },
    style: {
      get: function() {
        var nid = this.__oasis_node_id;
        // Proxy-like object: direct property access (e.g. .color)
        // maps to CSS property names via camelCase-to-kebab conversion.
        return new Proxy({
          setProperty: function(p, v) {
            __oasis_style_set(nid, p, String(v));
          },
          getPropertyValue: function(p) {
            return __oasis_style_get(nid, p);
          }
        }, {
          set: function(target, prop, value) {
            if (typeof prop === 'string') {
              var css_prop = prop.replace(
                /[A-Z]/g,
                function(m) { return '-' + m.toLowerCase(); }
              );
              __oasis_style_set(nid, css_prop, String(value));
            }
            return true;
          },
          get: function(target, prop) {
            if (typeof target[prop] === 'function') return target[prop];
            if (typeof prop === 'string') {
              var css_prop = prop.replace(
                /[A-Z]/g,
                function(m) { return '-' + m.toLowerCase(); }
              );
              return __oasis_style_get(nid, css_prop);
            }
            return undefined;
          }
        });
      },
      enumerable: true
    }
  });

  Element.prototype.getAttribute = function(name) {
    var v = __oasis_getattr(this.__oasis_node_id, name);
    return v === undefined ? null : v;
  };
  Element.prototype.setAttribute = function(name, value) {
    __oasis_setattr(this.__oasis_node_id, name, String(value));
  };
  Element.prototype.removeAttribute = function(name) {
    __oasis_rmattr(this.__oasis_node_id, name);
  };
  Element.prototype.appendChild = function(child) {
    __oasis_append(
      this.__oasis_node_id, child.__oasis_node_id
    );
    return child;
  };
  Element.prototype.removeChild = function(child) {
    __oasis_remove(child.__oasis_node_id);
    return child;
  };
  Element.prototype.insertBefore = function(newNode, refNode) {
    var refId = refNode ? refNode.__oasis_node_id : -1;
    __oasis_insertbefore(
      this.__oasis_node_id,
      newNode.__oasis_node_id,
      refId
    );
    return newNode;
  };
  Element.prototype.querySelector = function(sel) {
    var nid = __oasis_query_selector(
      this.__oasis_node_id, sel
    );
    return __get_el(nid);
  };
  Element.prototype.querySelectorAll = function(sel) {
    var nids = __oasis_query_selector_all(
      this.__oasis_node_id, sel
    );
    return nids.map(function(n) { return new Element(n); });
  };

  // -- Event listener support --
  // Listeners stored as {fn, once, capture, passive} objects.
  var __oasis_listeners = {};

  function __parse_opts(opts) {
    var c = false, o = false, p = false;
    if (opts === true || opts === false) { c = opts; }
    else if (opts && typeof opts === 'object') {
      c = !!opts.capture; o = !!opts.once; p = !!opts.passive;
    }
    return {capture: c, once: o, passive: p};
  }

  Element.prototype.addEventListener = function(type, fn, opts) {
    if (!fn) return;
    var o = __parse_opts(opts);
    var nid = this.__oasis_node_id;
    var key = nid + ":" + type;
    if (!__oasis_listeners[key]) __oasis_listeners[key] = [];
    var arr = __oasis_listeners[key];
    for (var i = 0; i < arr.length; i++) {
      if (arr[i].fn === fn && arr[i].capture === o.capture) return;
    }
    arr.push({fn: fn, once: o.once, capture: o.capture, passive: o.passive});
  };
  Element.prototype.removeEventListener = function(type, fn, opts) {
    var cap = false;
    if (opts === true || opts === false) cap = opts;
    else if (opts && typeof opts === 'object') cap = !!opts.capture;
    var nid = this.__oasis_node_id;
    var key = nid + ":" + type;
    var arr = __oasis_listeners[key];
    if (!arr) return;
    for (var i = 0; i < arr.length; i++) {
      if (arr[i].fn === fn && arr[i].capture === cap) {
        arr.splice(i, 1); return;
      }
    }
  };
  Element.prototype.dispatchEvent = function(evt) {
    var nid = this.__oasis_node_id;
    var key = nid + ":" + evt.type;
    var arr = __oasis_listeners[key];
    if (!arr) return;
    evt.target = this;
    for (var i = 0; i < arr.length; i++) {
      arr[i].fn.call(this, evt);
      if (arr[i] && arr[i].once) { arr.splice(i, 1); i--; }
    }
  };

  // Helper: invoke matching listeners, handling once removal.
  // phase: 1=capture, 2=target, 3=bubble
  function __fire(key, el, evt, phase) {
    var arr = __oasis_listeners[key];
    if (!arr) return;
    for (var i = 0; i < arr.length; i++) {
      if (evt._stopped) break;
      var e = arr[i];
      if (phase === 2 || (phase === 1 && e.capture) ||
          (phase === 3 && !e.capture)) {
        evt.currentTarget = el;
        e.fn.call(el, evt);
        if (e.once) { arr.splice(i, 1); i--; }
      }
    }
  }

  // Expose dispatch helper for Rust-side event triggering.
  globalThis.__oasis_dispatch_event =
    function(nid, type, detail) {
      var key = nid + ":" + type;
      var arr = __oasis_listeners[key];
      if (!arr || arr.length === 0) return;
      var el = new Element(nid);
      var evt = {
        type: type, target: el, detail: detail || null,
        _stopped: false,
        stopPropagation: function() { this._stopped = true; },
        preventDefault: function() { this._defaultPrevented = true; },
        _defaultPrevented: false
      };
      for (var i = 0; i < arr.length; i++) {
        arr[i].fn.call(el, evt);
        if (arr[i] && arr[i].once) { arr.splice(i, 1); i--; }
      }
    };

  // Dispatch with capture, target, and bubble phases.
  globalThis.__oasis_dispatch_with_bubbling =
    function(nid, type, detail) {
      var target = new Element(nid);
      var evt = {
        type: type,
        detail: detail || null,
        target: target,
        currentTarget: null,
        eventPhase: 0,
        _stopped: false,
        stopPropagation: function() {
          this._stopped = true;
        },
        preventDefault: function() {
          this._defaultPrevented = true;
        },
        _defaultPrevented: false
      };
      if (detail && typeof detail === 'object') {
        for (var k in detail) {
          if (detail.hasOwnProperty(k)) evt[k] = detail[k];
        }
      }
      // Build ancestor chain (excluding target), root first.
      var ancestors = [];
      var p = __oasis_parent(nid);
      while (p >= 0) { ancestors.push(p); p = __oasis_parent(p); }
      ancestors.reverse();
      // Capture phase: root -> target (ancestors only, capture listeners).
      evt.eventPhase = 1;
      for (var i = 0; i < ancestors.length && !evt._stopped; i++) {
        __fire(ancestors[i] + ":" + type, new Element(ancestors[i]), evt, 1);
      }
      // Target phase: all listeners on target.
      if (!evt._stopped) {
        evt.eventPhase = 2;
        __fire(nid + ":" + type, target, evt, 2);
      }
      // Bubble phase: target -> root (ancestors only, non-capture listeners).
      evt.eventPhase = 3;
      for (var i = ancestors.length - 1; i >= 0 && !evt._stopped; i--) {
        __fire(ancestors[i] + ":" + type, new Element(ancestors[i]), evt, 3);
      }
      // Report default-prevented back to the Rust side so it can skip
      // follow-up behaviors like link navigation when the page says
      // "return false" from an inline onclick handler.
      return evt._defaultPrevented;
    };

  // Thin dispatch helpers called from Rust via `Function::call` on the
  // already-compiled JS function (see `dispatch_js_event_fast` in
  // widget/input.rs). These wrappers exist so the hot click/mousemove/
  // keydown paths don't have to `format!` a JS source string and invoke
  // `engine.eval()` — which parses and compiles the snippet every time.
  // Parsing each event's fresh source string on a link-dense page (the
  // reddit sidebar, a nested comment thread, scrolling through a
  // listing with hover listeners) is measurable.
  globalThis.__oasis_dispatch_click_fast = function(nid, type) {
    return !!__oasis_dispatch_with_bubbling(nid, type, null);
  };
  globalThis.__oasis_dispatch_mouse_fast = function(nid, type, x, y) {
    __oasis_dispatch_with_bubbling(nid, type, {clientX: x, clientY: y});
  };
  globalThis.__oasis_dispatch_key_fast = function(nid, type, key, code) {
    __oasis_dispatch_with_bubbling(nid, type, {key: key, code: code});
  };

  var document = {
    getElementById: function(id) {
      var nid = __oasis_getbyid(id);
      return nid >= 0 ? new Element(nid) : null;
    },
    createElement: function(tag) {
      return new Element(__oasis_create(tag));
    },
    createTextNode: function(text) {
      return new Element(__oasis_createtext(String(text)));
    },
    querySelector: function(sel) {
      var b = __oasis_body();
      if (b < 0) return null;
      var nid = __oasis_query_selector(b, sel);
      return __get_el(nid);
    },
    querySelectorAll: function(sel) {
      var b = __oasis_body();
      if (b < 0) return [];
      var nids = __oasis_query_selector_all(b, sel);
      return nids.map(function(n) {
        return new Element(n);
      });
    }
  };

  Object.defineProperties(document, {
    body: {
      get: function() {
        var nid = __oasis_body();
        return nid >= 0 ? new Element(nid) : null;
      },
      enumerable: true
    },
    title: {
      get: function() { return __oasis_title(); },
      set: function(v) { __oasis_settitle(String(v)); },
      enumerable: true
    }
  });

  // Give document event listener support.
  var __doc_listeners = {};
  document.addEventListener = function(type, fn, opts) {
    if (!fn) return;
    var o = __parse_opts(opts);
    if (!__doc_listeners[type]) __doc_listeners[type] = [];
    var arr = __doc_listeners[type];
    for (var i = 0; i < arr.length; i++) {
      if (arr[i].fn === fn && arr[i].capture === o.capture) return;
    }
    arr.push({fn: fn, once: o.once, capture: o.capture, passive: o.passive});
  };
  document.removeEventListener = function(type, fn, opts) {
    var cap = false;
    if (opts === true || opts === false) cap = opts;
    else if (opts && typeof opts === 'object') cap = !!opts.capture;
    if (!__doc_listeners[type]) return;
    __doc_listeners[type] = __doc_listeners[type].filter(function(e) {
      return !(e.fn === fn && e.capture === cap);
    });
  };
  document.dispatchEvent = function(evt) {
    var type = evt && evt.type ? evt.type : evt;
    var arr = __doc_listeners[type];
    if (!arr) return;
    for (var i = 0; i < arr.length; i++) {
      arr[i].fn(evt);
      if (arr[i] && arr[i].once) { arr.splice(i, 1); i--; }
    }
  };

  // Minimal Event constructor for DOMContentLoaded etc.
  if (typeof Event === 'undefined') {
    globalThis.Event = function(type) { this.type = type; };
  }

  globalThis.document = document;
  globalThis.Element = Element;
  globalThis.window = globalThis;

  // -- location object with assign() and href setter --
  var __oasis_loc = {
    get href() { return __oasis_location(); },
    set href(v) { __oasis_location_assign(String(v)); },
    assign: function(url) { __oasis_location_assign(String(url)); },
    replace: function(url) { __oasis_location_assign(String(url)); },
    reload: function() { __oasis_location_assign(__oasis_location()); },
    toString: function() { return __oasis_location(); }
  };
  Object.defineProperty(globalThis, 'location', {
    get: function() { return __oasis_loc; },
    set: function(v) { __oasis_location_assign(String(v)); },
    configurable: true
  });

  // -- history object --
  globalThis.history = {
    __state: null,
    back: function() { __oasis_history_back(); },
    forward: function() { __oasis_history_forward(); },
    go: function(delta) {
      if (delta < 0) __oasis_history_back();
      else if (delta > 0) __oasis_history_forward();
      else __oasis_location_assign(__oasis_location());
    },
    pushState: function(state, title, url) {
      this.__state = state;
      if (url) __oasis_location_push(String(url));
    },
    replaceState: function(state, title, url) {
      this.__state = state;
      if (url) __oasis_location_push(String(url));
    },
    get state() { return this.__state; },
    get length() { return 1; }
  };

  // -- fetch API (synchronous under the hood) --
  globalThis.fetch = function(url, options) {
    var method = (options && options.method) || "GET";
    var reqBody = (options && options.body) || "";
    var body = __oasis_fetch(method, String(url), String(reqBody));
    return {
      then: function(fn) {
        var result = fn({
          ok: body.length > 0,
          status: body.length > 0 ? 200 : 0,
          text: function() { return { then: function(f) { return f(body); } }; },
          json: function() { return { then: function(f) { return f(JSON.parse(body)); } }; }
        });
        return { then: function(f) { return f ? f(result) : result; }, catch: function() { return this; } };
      },
      catch: function(fn) { return this; }
    };
  };

  // -- getComputedStyle --
  globalThis.getComputedStyle = function(el) {
    return {
      getPropertyValue: function(prop) {
        return __oasis_computed_style(el.__oasis_node_id, prop);
      }
    };
  };

  // -- localStorage / sessionStorage --
  // kind: 0 = localStorage, 1 = sessionStorage (separate backing stores)
  var __make_storage = function(kind) {
    return {
      getItem: function(k) { var v = __oasis_storage_get(kind, String(k)); return v === "" ? null : v; },
      setItem: function(k, v) { __oasis_storage_set(kind, String(k), String(v)); },
      removeItem: function(k) { __oasis_storage_remove(kind, String(k)); },
      clear: function() { __oasis_storage_clear(kind); },
      get length() { return __oasis_storage_length(kind); }
    };
  };
  globalThis.localStorage = __make_storage(0);
  globalThis.sessionStorage = __make_storage(1);

  // -- document.cookie --
  Object.defineProperty(document, 'cookie', {
    get: function() { return __oasis_cookie_get(); },
    set: function(v) { __oasis_cookie_set(String(v)); },
    configurable: true
  });
})();
