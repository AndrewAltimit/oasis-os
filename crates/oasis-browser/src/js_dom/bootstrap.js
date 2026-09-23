(function() {
  "use strict";

  if (typeof globalThis.DOMException === 'undefined') {
    globalThis.DOMException = function DOMException(message, name) {
      var e = new Error(message);
      e.name = name || 'Error';
      Object.setPrototypeOf(e, DOMException.prototype);
      return e;
    };
    DOMException.prototype = Object.create(Error.prototype);
    DOMException.prototype.constructor = DOMException;
  }

  // -- Wrapper cache -------------------------------------------------
  // One wrapper object per node id, so identity comparisons
  // (`a.parentNode === b`) and expando properties work. Entries are
  // evicted when the Rust side frees a node's arena slot (innerHTML /
  // textContent writes), because the slot may be reused for a new node.
  var __cache = Object.create(null);
  // Event listeners: key (node id, or 'w' for window) -> type -> entries.
  var __listeners = Object.create(null);
  var __ROOT = __oasis_root();

  function __wrap(nid) {
    if (typeof nid !== 'number' || nid < 0) return null;
    var w = __cache[nid];
    if (w !== undefined) return w;
    var proto;
    switch (__oasis_node_type(nid)) {
      case 1: proto = Element.prototype; break;
      case 3: proto = Text.prototype; break;
      case 8: proto = Comment.prototype; break;
      case 9: return document;
      case 11: proto = DocumentFragment.prototype; break;
      default: return null;
    }
    w = Object.create(proto);
    Object.defineProperty(w, '__oasis_node_id', {value: nid});
    __cache[nid] = w;
    return w;
  }

  // Array subclass standing in for NodeList / HTMLCollection.
  var NodeListProto = Object.create(Array.prototype);
  Object.defineProperty(NodeListProto, 'item', {
    value: function(i) { var n = this[i]; return n === undefined ? null : n; }
  });
  function __wrap_all(ids) {
    var out = [];
    for (var i = 0; i < ids.length; i++) {
      var w = __wrap(ids[i]);
      if (w) out.push(w);
    }
    Object.setPrototypeOf(out, NodeListProto);
    return out;
  }

  // Drop wrappers and listeners for arena slots the Rust side freed.
  function __sync_freed() {
    var ids = __oasis_take_freed();
    for (var i = 0; i < ids.length; i++) {
      delete __cache[ids[i]];
      delete __listeners[ids[i]];
    }
  }

  function __id(n) {
    if (!n || typeof n.__oasis_node_id !== 'number') {
      throw new TypeError("parameter is not of type 'Node'");
    }
    return n.__oasis_node_id;
  }

  function __check(rc) {
    if (rc === -1) {
      throw new DOMException(
        'The new child element contains the parent.', 'HierarchyRequestError');
    }
    if (rc === -2) throw new DOMException('Invalid node.', 'NotFoundError');
  }

  function __as_node(n) {
    if (n && typeof n.__oasis_node_id === 'number') return n;
    return document.createTextNode(String(n));
  }

  function __props(proto, map) {
    for (var k in map) {
      var d = map[k];
      d.enumerable = true;
      d.configurable = true;
      Object.defineProperty(proto, k, d);
    }
  }

  // -- Node hierarchy -------------------------------------------------
  // Constructors resolve to the cached wrapper, so the legacy
  // `new Element(nid)` form still yields the canonical object.
  function Node(nid) { return __wrap(nid); }
  function CharacterData(nid) { return __wrap(nid); }
  function Text(nid) { return __wrap(nid); }
  function Comment(nid) { return __wrap(nid); }
  function Element(nid) { return __wrap(nid); }
  function DocumentFragment(nid) { return __wrap(nid); }
  function Document(nid) { return __wrap(nid); }
  CharacterData.prototype = Object.create(Node.prototype);
  Text.prototype = Object.create(CharacterData.prototype);
  Comment.prototype = Object.create(CharacterData.prototype);
  Element.prototype = Object.create(Node.prototype);
  DocumentFragment.prototype = Object.create(Node.prototype);
  Document.prototype = Object.create(Node.prototype);
  [CharacterData, Text, Comment, Element, DocumentFragment, Document].forEach(function(C) {
    Object.defineProperty(C.prototype, 'constructor', {value: C, writable: true});
  });

  var NODE_CONSTS = {
    ELEMENT_NODE: 1, ATTRIBUTE_NODE: 2, TEXT_NODE: 3, COMMENT_NODE: 8,
    DOCUMENT_NODE: 9, DOCUMENT_TYPE_NODE: 10, DOCUMENT_FRAGMENT_NODE: 11
  };
  for (var nk in NODE_CONSTS) {
    Node[nk] = NODE_CONSTS[nk];
    Node.prototype[nk] = NODE_CONSTS[nk];
  }

  __props(Node.prototype, {
    nodeType: {
      get: function() { return __oasis_node_type(this.__oasis_node_id); }
    },
    nodeName: {
      get: function() {
        switch (this.nodeType) {
          case 1: return __oasis_tagname(this.__oasis_node_id);
          case 3: return '#text';
          case 8: return '#comment';
          case 9: return '#document';
          case 11: return '#document-fragment';
        }
        return '';
      }
    },
    nodeValue: {
      get: function() {
        var v = __oasis_node_value(this.__oasis_node_id);
        return v === undefined ? null : v;
      },
      set: function(v) {
        var t = this.nodeType;
        if (t === 3 || t === 8) {
          __oasis_set_node_value(this.__oasis_node_id, v == null ? '' : String(v));
        }
      }
    },
    textContent: {
      get: function() {
        var t = this.nodeType;
        if (t === 9) return null;
        if (t === 3 || t === 8) return this.nodeValue;
        return __oasis_text(this.__oasis_node_id);
      },
      set: function(v) {
        if (this.nodeType === 9) return;
        __oasis_settext(this.__oasis_node_id, v == null ? '' : String(v));
        __sync_freed();
      }
    },
    parentNode: {
      get: function() { return __wrap(__oasis_parent_node(this.__oasis_node_id)); }
    },
    parentElement: {
      get: function() {
        var p = __oasis_parent_node(this.__oasis_node_id);
        return p >= 0 && __oasis_node_type(p) === 1 ? __wrap(p) : null;
      }
    },
    childNodes: {
      get: function() { return __wrap_all(__oasis_child_nodes(this.__oasis_node_id)); }
    },
    firstChild: {
      get: function() {
        var ids = __oasis_child_nodes(this.__oasis_node_id);
        return ids.length ? __wrap(ids[0]) : null;
      }
    },
    lastChild: {
      get: function() {
        var ids = __oasis_child_nodes(this.__oasis_node_id);
        return ids.length ? __wrap(ids[ids.length - 1]) : null;
      }
    },
    nextSibling: {
      get: function() { return __wrap(__oasis_sibling(this.__oasis_node_id, 1, false)); }
    },
    previousSibling: {
      get: function() { return __wrap(__oasis_sibling(this.__oasis_node_id, -1, false)); }
    },
    isConnected: {
      get: function() { return __oasis_is_connected(this.__oasis_node_id); }
    },
    ownerDocument: {
      get: function() { return this === document ? null : document; }
    }
  });

  Node.prototype.hasChildNodes = function() {
    return __oasis_child_nodes(this.__oasis_node_id).length > 0;
  };
  Node.prototype.appendChild = function(child) {
    __check(__oasis_append(this.__oasis_node_id, __id(child)));
    return child;
  };
  Node.prototype.insertBefore = function(newNode, refNode) {
    var refId = refNode ? __id(refNode) : -1;
    __check(__oasis_insertbefore(this.__oasis_node_id, __id(newNode), refId));
    return newNode;
  };
  Node.prototype.removeChild = function(child) {
    if (__oasis_parent_node(__id(child)) !== this.__oasis_node_id) {
      throw new DOMException(
        'The node to be removed is not a child of this node.', 'NotFoundError');
    }
    __oasis_remove(child.__oasis_node_id);
    return child;
  };
  Node.prototype.replaceChild = function(newChild, oldChild) {
    if (__oasis_parent_node(__id(oldChild)) !== this.__oasis_node_id) {
      throw new DOMException(
        'The node to be replaced is not a child of this node.', 'NotFoundError');
    }
    if (newChild === oldChild) return oldChild;
    this.insertBefore(newChild, oldChild);
    __oasis_remove(oldChild.__oasis_node_id);
    return oldChild;
  };
  Node.prototype.cloneNode = function(deep) {
    return __wrap(__oasis_clone(this.__oasis_node_id, !!deep));
  };
  Node.prototype.contains = function(other) {
    if (!other || typeof other.__oasis_node_id !== 'number') return false;
    var target = this.__oasis_node_id;
    var n = other.__oasis_node_id;
    while (n >= 0) {
      if (n === target) return true;
      n = __oasis_parent_node(n);
    }
    return false;
  };
  Node.prototype.isSameNode = function(other) { return this === other; };
  Node.prototype.getRootNode = function() {
    var n = this.__oasis_node_id;
    var p = __oasis_parent_node(n);
    while (p >= 0) { n = p; p = __oasis_parent_node(n); }
    return __wrap(n);
  };

  // -- CharacterData (Text / Comment) ----------------------------------
  __props(CharacterData.prototype, {
    data: {
      get: function() { return this.nodeValue; },
      set: function(v) { this.nodeValue = v; }
    },
    length: {
      get: function() { return this.nodeValue.length; }
    }
  });
  __props(Text.prototype, {
    wholeText: { get: function() { return this.nodeValue; } }
  });

  // -- ChildNode mixin (Element, Text, Comment) ------------------------
  var ChildNodeMixin = {
    remove: function() { __oasis_remove(this.__oasis_node_id); },
    before: function() {
      var parent = this.parentNode;
      if (!parent) return;
      for (var i = 0; i < arguments.length; i++) {
        parent.insertBefore(__as_node(arguments[i]), this);
      }
    },
    after: function() {
      var parent = this.parentNode;
      if (!parent) return;
      var ref = this.nextSibling;
      for (var i = 0; i < arguments.length; i++) {
        var n = __as_node(arguments[i]);
        if (n === ref) { ref = ref.nextSibling; continue; }
        parent.insertBefore(n, ref);
      }
    },
    replaceWith: function() {
      var parent = this.parentNode;
      if (!parent) return;
      var ref = this.nextSibling;
      var self = this;
      var keep = false;
      for (var i = 0; i < arguments.length; i++) {
        var n = __as_node(arguments[i]);
        if (n === self) { keep = true; continue; }
        if (n === ref) { ref = ref.nextSibling; continue; }
        parent.insertBefore(n, ref);
      }
      if (!keep) __oasis_remove(self.__oasis_node_id);
    }
  };
  var ChildNodeProps = {
    nextElementSibling: {
      get: function() { return __wrap(__oasis_sibling(this.__oasis_node_id, 1, true)); }
    },
    previousElementSibling: {
      get: function() { return __wrap(__oasis_sibling(this.__oasis_node_id, -1, true)); }
    }
  };
  [Element.prototype, CharacterData.prototype].forEach(function(p) {
    for (var k in ChildNodeMixin) p[k] = ChildNodeMixin[k];
    __props(p, {
      nextElementSibling: ChildNodeProps.nextElementSibling,
      previousElementSibling: ChildNodeProps.previousElementSibling
    });
  });

  // -- ParentNode mixin (Element, Document, DocumentFragment) ----------
  var ParentNodeMixin = {
    querySelector: function(sel) {
      return __wrap(__oasis_query_selector(this.__oasis_node_id, String(sel)));
    },
    querySelectorAll: function(sel) {
      return __wrap_all(__oasis_query_selector_all(this.__oasis_node_id, String(sel)));
    },
    getElementsByTagName: function(tag) {
      return __wrap_all(__oasis_by_tag(this.__oasis_node_id, String(tag)));
    },
    getElementsByClassName: function(names) {
      return __wrap_all(__oasis_by_class(this.__oasis_node_id, String(names)));
    },
    append: function() {
      for (var i = 0; i < arguments.length; i++) this.appendChild(__as_node(arguments[i]));
    },
    prepend: function() {
      var ref = this.firstChild;
      for (var i = 0; i < arguments.length; i++) {
        var n = __as_node(arguments[i]);
        if (n === ref) { ref = ref.nextSibling; continue; }
        this.insertBefore(n, ref);
      }
    },
    replaceChildren: function() {
      var kids = __oasis_child_nodes(this.__oasis_node_id);
      for (var k = 0; k < kids.length; k++) __oasis_remove(kids[k]);
      this.append.apply(this, arguments);
    }
  };
  var ParentNodeProps = {
    children: {
      get: function() { return __wrap_all(__oasis_children(this.__oasis_node_id)); }
    },
    firstElementChild: {
      get: function() {
        var ids = __oasis_children(this.__oasis_node_id);
        return ids.length ? __wrap(ids[0]) : null;
      }
    },
    lastElementChild: {
      get: function() {
        var ids = __oasis_children(this.__oasis_node_id);
        return ids.length ? __wrap(ids[ids.length - 1]) : null;
      }
    },
    childElementCount: {
      get: function() { return __oasis_children(this.__oasis_node_id).length; }
    }
  };
  [Element.prototype, Document.prototype, DocumentFragment.prototype].forEach(function(p) {
    for (var k in ParentNodeMixin) p[k] = ParentNodeMixin[k];
    __props(p, {
      children: ParentNodeProps.children,
      firstElementChild: ParentNodeProps.firstElementChild,
      lastElementChild: ParentNodeProps.lastElementChild,
      childElementCount: ParentNodeProps.childElementCount
    });
  });

  // -- DOMTokenList (classList) ----------------------------------------
  function DOMTokenList(el) {
    Object.defineProperty(this, '_nid', {value: el.__oasis_node_id});
  }
  function __tok_check(t) {
    t = String(t);
    if (t === '') throw new DOMException('The token provided must not be empty.', 'SyntaxError');
    if (/\s/.test(t)) {
      throw new DOMException(
        "The token provided ('" + t + "') contains whitespace.", 'InvalidCharacterError');
    }
    return t;
  }
  DOMTokenList.prototype._get = function() {
    var c = __oasis_getattr(this._nid, 'class');
    if (!c) return [];
    return c.split(/\s+/).filter(function(x) { return x.length > 0; });
  };
  DOMTokenList.prototype._set = function(tokens) {
    __oasis_setattr(this._nid, 'class', tokens.join(' '));
  };
  DOMTokenList.prototype.contains = function(t) {
    return this._get().indexOf(String(t)) !== -1;
  };
  DOMTokenList.prototype.add = function() {
    var toks = this._get(), changed = false;
    for (var i = 0; i < arguments.length; i++) {
      var t = __tok_check(arguments[i]);
      if (toks.indexOf(t) === -1) { toks.push(t); changed = true; }
    }
    if (changed) this._set(toks);
  };
  DOMTokenList.prototype.remove = function() {
    var toks = this._get(), before = toks.length;
    for (var i = 0; i < arguments.length; i++) {
      var t = __tok_check(arguments[i]);
      toks = toks.filter(function(x) { return x !== t; });
    }
    if (toks.length !== before) this._set(toks);
  };
  DOMTokenList.prototype.toggle = function(t, force) {
    t = __tok_check(t);
    var has = this.contains(t);
    if (force === true || (force === undefined && !has)) {
      if (!has) this.add(t);
      return true;
    }
    if (has) this.remove(t);
    return false;
  };
  DOMTokenList.prototype.replace = function(oldT, newT) {
    oldT = __tok_check(oldT);
    newT = __tok_check(newT);
    var toks = this._get();
    var idx = toks.indexOf(oldT);
    if (idx === -1) return false;
    if (toks.indexOf(newT) !== -1 && newT !== oldT) toks.splice(idx, 1);
    else toks[idx] = newT;
    this._set(toks);
    return true;
  };
  DOMTokenList.prototype.item = function(i) {
    var t = this._get()[i];
    return t === undefined ? null : t;
  };
  DOMTokenList.prototype.forEach = function(cb, thisArg) {
    this._get().forEach(cb, thisArg);
  };
  DOMTokenList.prototype.toString = function() { return this.value; };
  DOMTokenList.prototype[Symbol.iterator] = function() {
    return this._get()[Symbol.iterator]();
  };
  __props(DOMTokenList.prototype, {
    length: { get: function() { return this._get().length; } },
    value: {
      get: function() { return __oasis_getattr(this._nid, 'class') || ''; },
      set: function(v) { __oasis_setattr(this._nid, 'class', String(v)); }
    }
  });

  // -- Element ---------------------------------------------------------
  function __data_attr(prop) {
    return 'data-' + prop.replace(/[A-Z]/g, function(m) { return '-' + m.toLowerCase(); });
  }
  function __data_prop(attr) {
    return attr.slice(5).replace(/-([a-z])/g, function(m, c) { return c.toUpperCase(); });
  }
  function __make_dataset(el) {
    var nid = el.__oasis_node_id;
    function read(p) {
      if (typeof p !== 'string') return undefined;
      return __oasis_getattr(nid, __data_attr(p));
    }
    return new Proxy({}, {
      get: function(t, p) { return read(p); },
      set: function(t, p, v) {
        if (typeof p === 'string') el.setAttribute(__data_attr(p), v);
        return true;
      },
      has: function(t, p) { return read(p) !== undefined; },
      deleteProperty: function(t, p) {
        if (typeof p === 'string') el.removeAttribute(__data_attr(p));
        return true;
      },
      ownKeys: function() {
        var a = __oasis_attrs(nid), keys = [];
        for (var i = 0; i < a.length; i += 2) {
          if (a[i].indexOf('data-') === 0) keys.push(__data_prop(a[i]));
        }
        return keys;
      },
      getOwnPropertyDescriptor: function(t, p) {
        var v = read(p);
        if (v === undefined) return undefined;
        return {value: v, writable: true, enumerable: true, configurable: true};
      }
    });
  }

  function __make_style(nid) {
    // Proxy-like object: direct property access (e.g. .color)
    // maps to CSS property names via camelCase-to-kebab conversion.
    function kebab(prop) {
      return prop.replace(/[A-Z]/g, function(m) { return '-' + m.toLowerCase(); });
    }
    return new Proxy({
      setProperty: function(p, v) { __oasis_style_set(nid, p, String(v)); },
      getPropertyValue: function(p) { return __oasis_style_get(nid, p); }
    }, {
      set: function(target, prop, value) {
        if (prop === 'cssText') __oasis_setattr(nid, 'style', String(value));
        else if (typeof prop === 'string') __oasis_style_set(nid, kebab(prop), String(value));
        return true;
      },
      get: function(target, prop) {
        if (typeof target[prop] === 'function') return target[prop];
        if (prop === 'cssText') return __oasis_getattr(nid, 'style') || '';
        if (typeof prop === 'string') return __oasis_style_get(nid, kebab(prop));
        return undefined;
      }
    });
  }

  // Cached helper objects (classList / dataset / style) are stashed on
  // the wrapper as non-enumerable properties so `el.classList ===
  // el.classList`.
  function __stash(el, key, make) {
    var v = el[key];
    if (v === undefined) {
      v = make();
      Object.defineProperty(el, key, {value: v});
    }
    return v;
  }

  function __local(el) { return __oasis_tagname(el.__oasis_node_id).toLowerCase(); }

  function __options(el) { return el.getElementsByTagName('option'); }
  function __selected_index(el) {
    var opts = __options(el);
    for (var i = 0; i < opts.length; i++) {
      if (opts[i].hasAttribute('selected')) return i;
    }
    return opts.length ? 0 : -1;
  }

  __props(Element.prototype, {
    tagName: {
      get: function() { return __oasis_tagname(this.__oasis_node_id); }
    },
    localName: { get: function() { return __local(this); } },
    id: {
      get: function() { return __oasis_getattr(this.__oasis_node_id, 'id') || ''; },
      set: function(v) {
        if (v) __oasis_setattr(this.__oasis_node_id, 'id', String(v));
        else __oasis_rmattr(this.__oasis_node_id, 'id');
      }
    },
    className: {
      get: function() { return __oasis_getattr(this.__oasis_node_id, 'class') || ''; },
      set: function(v) { __oasis_setattr(this.__oasis_node_id, 'class', String(v)); }
    },
    classList: {
      get: function() {
        var self = this;
        return __stash(this, '__oasis_class_list', function() { return new DOMTokenList(self); });
      }
    },
    dataset: {
      get: function() {
        var self = this;
        return __stash(this, '__oasis_dataset', function() { return __make_dataset(self); });
      }
    },
    style: {
      get: function() {
        var nid = this.__oasis_node_id;
        return __stash(this, '__oasis_style', function() { return __make_style(nid); });
      }
    },
    attributes: {
      get: function() {
        var a = __oasis_attrs(this.__oasis_node_id), out = [];
        for (var i = 0; i < a.length; i += 2) out.push({name: a[i], value: a[i + 1]});
        out.getNamedItem = function(n) {
          for (var j = 0; j < this.length; j++) if (this[j].name === n) return this[j];
          return null;
        };
        return out;
      }
    },
    innerHTML: {
      get: function() { return __oasis_inner_html(this.__oasis_node_id); },
      set: function(v) {
        __oasis_set_inner_html(this.__oasis_node_id, v == null ? '' : String(v));
        __sync_freed();
      }
    },
    outerHTML: {
      get: function() { return __oasis_outer_html(this.__oasis_node_id); },
      set: function(v) {
        var p = __oasis_parent_node(this.__oasis_node_id);
        if (p < 0) return;
        var frag = __oasis_parse_fragment(String(v));
        __check(__oasis_insertbefore(p, frag, this.__oasis_node_id));
        __oasis_discard_fragment(frag);
        __oasis_remove(this.__oasis_node_id);
        __sync_freed();
      }
    },
    innerText: {
      get: function() { return this.textContent; },
      set: function(v) { this.textContent = v; }
    },
    // -- Form control reflection --
    value: {
      get: function() {
        var tag = __local(this);
        if (tag === 'select') {
          var opts = __options(this), i = __selected_index(this);
          return i >= 0 ? opts[i].value : '';
        }
        var v = this.getAttribute('value');
        if (tag === 'textarea') return v !== null ? v : this.textContent;
        if (tag === 'option') return v !== null ? v : this.textContent.trim();
        if (v !== null) return v;
        var type = (this.getAttribute('type') || '').toLowerCase();
        return type === 'checkbox' || type === 'radio' ? 'on' : '';
      },
      set: function(v) {
        v = v == null ? '' : String(v);
        var tag = __local(this);
        if (tag === 'select') {
          var opts = __options(this), found = false;
          for (var i = 0; i < opts.length; i++) {
            var hit = !found && opts[i].value === v;
            opts[i].toggleAttribute('selected', hit);
            if (hit) found = true;
          }
          return;
        }
        this.setAttribute('value', v);
        if (tag === 'textarea') this.textContent = v;
      }
    },
    selectedIndex: {
      get: function() { return __selected_index(this); },
      set: function(idx) {
        var opts = __options(this);
        for (var i = 0; i < opts.length; i++) opts[i].toggleAttribute('selected', i === idx);
      }
    },
    options: { get: function() { return __options(this); } },
    checked: {
      get: function() { return this.hasAttribute('checked'); },
      set: function(v) {
        v = !!v;
        var name = this.getAttribute('name');
        if (v && name && (this.getAttribute('type') || '').toLowerCase() === 'radio') {
          var inputs = document.getElementsByTagName('input');
          for (var i = 0; i < inputs.length; i++) {
            var r = inputs[i];
            if (r !== this && r.getAttribute('name') === name &&
                (r.getAttribute('type') || '').toLowerCase() === 'radio') {
              r.removeAttribute('checked');
            }
          }
        }
        this.toggleAttribute('checked', v);
      }
    },
    type: {
      get: function() {
        var t = this.getAttribute('type');
        if (t) return t.toLowerCase();
        var tag = __local(this);
        if (tag === 'input') return 'text';
        if (tag === 'button') return 'submit';
        if (tag === 'select') return this.hasAttribute('multiple') ? 'select-multiple' : 'select-one';
        if (tag === 'textarea') return 'textarea';
        return '';
      },
      set: function(v) { this.setAttribute('type', v); }
    },
    htmlFor: {
      get: function() { return this.getAttribute('for') || ''; },
      set: function(v) { this.setAttribute('for', v); }
    }
  });
  // Plain string / boolean attribute reflection.
  ['name', 'href', 'src', 'title', 'alt', 'placeholder', 'rel', 'target', 'lang',
   'action', 'method'].forEach(function(attr) {
    var d = {};
    d[attr] = {
      get: function() { return this.getAttribute(attr) || ''; },
      set: function(v) { this.setAttribute(attr, v); }
    };
    __props(Element.prototype, d);
  });
  [['disabled', 'disabled'], ['selected', 'selected'], ['hidden', 'hidden'],
   ['required', 'required'], ['readOnly', 'readonly'], ['multiple', 'multiple']
  ].forEach(function(pair) {
    var d = {};
    d[pair[0]] = {
      get: function() { return this.hasAttribute(pair[1]); },
      set: function(v) { this.toggleAttribute(pair[1], !!v); }
    };
    __props(Element.prototype, d);
  });

  Element.prototype.getAttribute = function(name) {
    var v = __oasis_getattr(this.__oasis_node_id, String(name));
    return v === undefined ? null : v;
  };
  Element.prototype.setAttribute = function(name, value) {
    __oasis_setattr(this.__oasis_node_id, String(name), String(value));
  };
  Element.prototype.removeAttribute = function(name) {
    __oasis_rmattr(this.__oasis_node_id, String(name));
  };
  Element.prototype.hasAttribute = function(name) {
    return __oasis_getattr(this.__oasis_node_id, String(name)) !== undefined;
  };
  Element.prototype.hasAttributes = function() {
    return __oasis_attrs(this.__oasis_node_id).length > 0;
  };
  Element.prototype.getAttributeNames = function() {
    var a = __oasis_attrs(this.__oasis_node_id), out = [];
    for (var i = 0; i < a.length; i += 2) out.push(a[i]);
    return out;
  };
  Element.prototype.toggleAttribute = function(name, force) {
    var has = this.hasAttribute(name);
    if (force === true || (force === undefined && !has)) {
      if (!has) this.setAttribute(name, '');
      return true;
    }
    if (has) this.removeAttribute(name);
    return false;
  };
  Element.prototype.matches = function(sel) {
    var r = __oasis_matches(this.__oasis_node_id, String(sel));
    if (r < 0) {
      throw new DOMException("'" + sel + "' is not a valid selector.", 'SyntaxError');
    }
    return r === 1;
  };
  Element.prototype.webkitMatchesSelector = Element.prototype.matches;
  Element.prototype.msMatchesSelector = Element.prototype.matches;
  Element.prototype.closest = function(sel) {
    sel = String(sel);
    var n = this.__oasis_node_id;
    while (n >= 0 && __oasis_node_type(n) === 1) {
      var r = __oasis_matches(n, sel);
      if (r < 0) {
        throw new DOMException("'" + sel + "' is not a valid selector.", 'SyntaxError');
      }
      if (r === 1) return __wrap(n);
      n = __oasis_parent_node(n);
    }
    return null;
  };
  function __adjacent(el, where, nodeId) {
    var nid = el.__oasis_node_id;
    var parent = __oasis_parent_node(nid);
    switch (String(where).toLowerCase()) {
      case 'beforebegin':
        if (parent < 0) return false;
        __check(__oasis_insertbefore(parent, nodeId, nid));
        return true;
      case 'afterbegin':
        var first = __oasis_child_nodes(nid);
        __check(__oasis_insertbefore(nid, nodeId, first.length ? first[0] : -1));
        return true;
      case 'beforeend':
        __check(__oasis_append(nid, nodeId));
        return true;
      case 'afterend':
        if (parent < 0) return false;
        __check(__oasis_insertbefore(parent, nodeId, __oasis_sibling(nid, 1, false)));
        return true;
    }
    throw new DOMException(
      "The value provided ('" + where + "') is not one of 'beforeBegin', 'afterBegin', " +
      "'beforeEnd', or 'afterEnd'.", 'SyntaxError');
  }
  Element.prototype.insertAdjacentHTML = function(where, html) {
    var frag = __oasis_parse_fragment(String(html));
    try {
      __adjacent(this, where, frag);
    } finally {
      __oasis_discard_fragment(frag);
      __sync_freed();
    }
  };
  Element.prototype.insertAdjacentElement = function(where, el) {
    return __adjacent(this, where, __id(el)) ? el : null;
  };
  Element.prototype.insertAdjacentText = function(where, text) {
    __adjacent(this, where, __id(document.createTextNode(String(text))));
  };
  Element.prototype.click = function() {
    this.dispatchEvent(new MouseEvent('click', {bubbles: true, cancelable: true}));
  };
  Element.prototype.focus = function() {};
  Element.prototype.blur = function() {};

  // -- Events ----------------------------------------------------------
  function Event(type, init) {
    init = init || {};
    this.type = String(type);
    this.bubbles = !!init.bubbles;
    this.cancelable = !!init.cancelable;
    this.composed = !!init.composed;
    this.defaultPrevented = false;
    this.target = null;
    this.srcElement = null;
    this.currentTarget = null;
    this.eventPhase = 0;
    this.isTrusted = false;
    this.cancelBubble = false;
    this.returnValue = true;
    this.timeStamp = Date.now();
    Object.defineProperty(this, '_stop', {value: false, writable: true});
    Object.defineProperty(this, '_stopImm', {value: false, writable: true});
    Object.defineProperty(this, '_path', {value: [], writable: true});
  }
  Event.NONE = 0;
  Event.CAPTURING_PHASE = 1;
  Event.AT_TARGET = 2;
  Event.BUBBLING_PHASE = 3;
  Event.prototype.stopPropagation = function() {
    this._stop = true;
    this.cancelBubble = true;
  };
  Event.prototype.stopImmediatePropagation = function() {
    this._stop = true;
    this._stopImm = true;
    this.cancelBubble = true;
  };
  Event.prototype.preventDefault = function() {
    if (this.cancelable) {
      this.defaultPrevented = true;
      this.returnValue = false;
    }
  };
  Event.prototype.composedPath = function() { return this._path.slice(); };
  Event.prototype.initEvent = function(type, bubbles, cancelable) {
    this.type = String(type);
    this.bubbles = !!bubbles;
    this.cancelable = !!cancelable;
  };

  // Event subclasses: copy recognised init-dict fields (with defaults).
  function __event_class(base, defaults) {
    function E(type, init) {
      base.call(this, type, init);
      init = init || {};
      for (var k in defaults) this[k] = init[k] !== undefined ? init[k] : defaults[k];
    }
    E.prototype = Object.create(base.prototype);
    E.prototype.constructor = E;
    return E;
  }
  var MODS = {ctrlKey: false, shiftKey: false, altKey: false, metaKey: false};
  function __with(a, b) {
    var o = {};
    for (var k in a) o[k] = a[k];
    for (var j in b) o[j] = b[j];
    return o;
  }
  var CustomEvent = __event_class(Event, {detail: null});
  CustomEvent.prototype.initCustomEvent = function(type, bubbles, cancelable, detail) {
    this.initEvent(type, bubbles, cancelable);
    this.detail = detail;
  };
  var UIEvent = __event_class(Event, {detail: 0, view: null});
  var MouseEvent = __event_class(UIEvent, __with(MODS, {
    clientX: 0, clientY: 0, screenX: 0, screenY: 0, pageX: 0, pageY: 0,
    offsetX: 0, offsetY: 0, button: 0, buttons: 0, relatedTarget: null
  }));
  var KeyboardEvent = __event_class(UIEvent, __with(MODS, {
    key: '', code: '', location: 0, repeat: false, isComposing: false
  }));
  var FocusEvent = __event_class(UIEvent, {relatedTarget: null});
  var InputEvent = __event_class(UIEvent, {data: null, inputType: '', isComposing: false});

  function __key(node) { return node === globalThis ? 'w' : node.__oasis_node_id; }

  function __parse_opts(opts) {
    var c = false, o = false, p = false;
    if (opts === true || opts === false) { c = opts; }
    else if (opts && typeof opts === 'object') {
      c = !!opts.capture; o = !!opts.once; p = !!opts.passive;
    }
    return {capture: c, once: o, passive: p};
  }

  function __add_listener(node, type, fn, opts) {
    if (!fn) return;
    var o = __parse_opts(opts);
    var key = __key(node);
    var map = __listeners[key] || (__listeners[key] = Object.create(null));
    var arr = map[type] || (map[type] = []);
    for (var i = 0; i < arr.length; i++) {
      if (arr[i].fn === fn && arr[i].capture === o.capture) return;
    }
    var entry = {fn: fn, once: o.once, capture: o.capture, passive: o.passive, removed: false};
    arr.push(entry);
    if (opts && typeof opts === 'object' && opts.signal &&
        typeof opts.signal.addEventListener === 'function') {
      opts.signal.addEventListener('abort', function() {
        __remove_listener(node, type, fn, o.capture);
      });
    }
  }

  function __remove_listener(node, type, fn, capture) {
    var map = __listeners[__key(node)];
    var arr = map && map[type];
    if (!arr) return;
    for (var i = 0; i < arr.length; i++) {
      if (arr[i].fn === fn && arr[i].capture === capture) {
        arr[i].removed = true;
        arr.splice(i, 1);
        return;
      }
    }
  }

  function __report(err) {
    try {
      if (typeof console !== 'undefined' && console.error) {
        console.error('Uncaught ' + (err && err.stack ? err + '\n' + err.stack : err));
      }
    } catch (e) { /* ignore */ }
  }

  function __call(fn, self, evt) {
    try {
      if (typeof fn === 'function') return fn.call(self, evt);
      if (fn && typeof fn.handleEvent === 'function') return fn.handleEvent(evt);
    } catch (err) {
      // A throwing listener must not stop the others (matches browsers).
      // Watchdog interrupts are uncatchable and still propagate.
      __report(err);
    }
    return undefined;
  }

  // phase: 1 = capture, 2 = at target, 3 = bubble.
  function __invoke(node, evt, phase) {
    var map = __listeners[__key(node)];
    var arr = map && map[evt.type];
    evt.currentTarget = node;
    evt.eventPhase = phase;
    if (arr && arr.length) {
      var snap = arr.slice();
      for (var i = 0; i < snap.length; i++) {
        var e = snap[i];
        if (e.removed) continue;
        if ((phase === 1 && !e.capture) || (phase === 3 && e.capture)) continue;
        if (e.once) __remove_listener(node, evt.type, e.fn, e.capture);
        __call(e.fn, node, evt);
        if (evt._stopImm) return;
      }
    }
    // `el.onclick = fn` / `window.onload = fn` style handlers.
    if (phase !== 1) {
      var h = node['on' + evt.type];
      if (typeof h === 'function' && __call(h, node, evt) === false) evt.preventDefault();
    }
  }

  function __dispatch(target, evt) {
    evt.target = target;
    evt.srcElement = target;
    evt._stop = false;
    evt._stopImm = false;
    // Ancestors, nearest first; connected nodes end at document, window.
    var path = [];
    if (target !== globalThis) {
      var p = __oasis_parent_node(target.__oasis_node_id);
      while (p >= 0) {
        var w = __wrap(p);
        if (w) path.push(w);
        p = __oasis_parent_node(p);
      }
      var top = path.length ? path[path.length - 1] : target;
      if (top === document && evt.type !== 'load') path.push(globalThis);
    }
    evt._path = [target].concat(path);
    var i;
    for (i = path.length - 1; i >= 0 && !evt._stop; i--) __invoke(path[i], evt, 1);
    if (!evt._stop) __invoke(target, evt, 2);
    if (evt.bubbles) {
      for (i = 0; i < path.length && !evt._stop; i++) __invoke(path[i], evt, 3);
    }
    evt.currentTarget = null;
    evt.eventPhase = 0;
    return !evt.defaultPrevented;
  }

  // Accept real Events, bare type strings and legacy `{type: ...}`
  // objects (given Event behaviour in place).
  function __as_event(evt) {
    if (typeof evt === 'string') return new Event(evt);
    if (!evt || typeof evt.type !== 'string') {
      throw new TypeError("Failed to execute 'dispatchEvent': parameter 1 is not an Event.");
    }
    if (!(evt instanceof Event)) {
      var proto = Event.prototype;
      ['stopPropagation', 'stopImmediatePropagation', 'preventDefault', 'composedPath']
        .forEach(function(m) { if (typeof evt[m] !== 'function') evt[m] = proto[m]; });
      if (evt.defaultPrevented === undefined) evt.defaultPrevented = false;
      if (evt.cancelable === undefined) evt.cancelable = true;
    }
    return evt;
  }

  Node.prototype.addEventListener = function(type, fn, opts) {
    __add_listener(this, String(type), fn, opts);
  };
  Node.prototype.removeEventListener = function(type, fn, opts) {
    __remove_listener(this, String(type), fn, __parse_opts(opts).capture);
  };
  Node.prototype.dispatchEvent = function(evt) {
    return __dispatch(this, __as_event(evt));
  };

  function __host_event(type, init) {
    if (/^(click|dblclick|contextmenu|mouse|pointer)/.test(type)) {
      return new MouseEvent(type, init);
    }
    if (/^key/.test(type)) return new KeyboardEvent(type, init);
    return new Event(type, init);
  }

  // Target-only (non-bubbling) dispatch helper for Rust callers.
  globalThis.__oasis_dispatch_event = function(nid, type, detail) {
    var target = __wrap(nid);
    if (!target) return false;
    var evt = __host_event(type, {cancelable: true});
    evt.isTrusted = true;
    evt.detail = detail === undefined ? null : detail;
    __dispatch(target, evt);
    return evt.defaultPrevented;
  };

  // Dispatch with capture, target, and bubble phases. Returns whether
  // the default action was prevented, so the Rust side can skip
  // follow-up behaviours like link navigation when the page says
  // "return false" from an inline onclick handler.
  globalThis.__oasis_dispatch_with_bubbling = function(nid, type, detail) {
    var target = __wrap(nid);
    if (!target) return false;
    var init = {bubbles: true, cancelable: true};
    if (detail && typeof detail === 'object') {
      for (var k in detail) {
        if (Object.prototype.hasOwnProperty.call(detail, k)) init[k] = detail[k];
      }
    }
    var evt = __host_event(type, init);
    evt.isTrusted = true;
    if (detail && typeof detail === 'object') {
      for (var j in detail) {
        if (Object.prototype.hasOwnProperty.call(detail, j)) evt[j] = detail[j];
      }
    }
    if (evt.detail === undefined || evt.detail === 0) evt.detail = detail || null;
    __dispatch(target, evt);
    return evt.defaultPrevented;
  };

  // Thin dispatch helpers called from Rust via `Function::call` on the
  // already-compiled JS function (see `dispatch_js_event_fast` in
  // widget/input.rs). These wrappers exist so the hot click/mousemove/
  // keydown paths don't have to `format!` a JS source string and invoke
  // `engine.eval()` — which parses and compiles the snippet every time.
  globalThis.__oasis_dispatch_click_fast = function(nid, type) {
    return !!__oasis_dispatch_with_bubbling(nid, type, null);
  };
  globalThis.__oasis_dispatch_mouse_fast = function(nid, type, x, y) {
    __oasis_dispatch_with_bubbling(nid, type, {clientX: x, clientY: y});
  };
  globalThis.__oasis_dispatch_key_fast = function(nid, type, key, code) {
    __oasis_dispatch_with_bubbling(nid, type, {key: key, code: code});
  };

  // -- document ----------------------------------------------------------
  var __ready_state = 'loading';
  var document = Object.create(Document.prototype);
  Object.defineProperty(document, '__oasis_node_id', {value: __ROOT});
  __cache[__ROOT] = document;

  document.getElementById = function(id) {
    return __wrap(__oasis_getbyid(String(id)));
  };
  document.getElementsByName = function(name) {
    name = String(name);
    return this.getElementsByTagName('*').filter(function(el) {
      return el.getAttribute('name') === name;
    });
  };
  document.createElement = function(tag) {
    return __wrap(__oasis_create(String(tag).toLowerCase()));
  };
  document.createElementNS = function(ns, qname) {
    var local = String(qname);
    var colon = local.indexOf(':');
    if (colon >= 0) local = local.slice(colon + 1);
    return __wrap(__oasis_create(local.toLowerCase()));
  };
  document.createTextNode = function(text) {
    return __wrap(__oasis_createtext(String(text)));
  };
  document.createComment = function(text) {
    return __wrap(__oasis_create_comment(String(text)));
  };
  document.createDocumentFragment = function() {
    return __wrap(__oasis_create_fragment());
  };
  document.createEvent = function(kind) {
    var k = String(kind).toLowerCase();
    if (k === 'customevent') return new CustomEvent('');
    if (k === 'mouseevent' || k === 'mouseevents') return new MouseEvent('');
    if (k === 'keyboardevent') return new KeyboardEvent('');
    return new Event('');
  };
  document.hasFocus = function() { return true; };

  __props(document, {
    documentElement: {
      get: function() {
        var ids = __oasis_children(__ROOT);
        return ids.length ? __wrap(ids[0]) : null;
      }
    },
    head: { get: function() { return __wrap(__oasis_head()); } },
    body: { get: function() { return __wrap(__oasis_body()); } },
    activeElement: { get: function() { return this.body; } },
    title: {
      get: function() { return __oasis_title(); },
      set: function(v) {
        __oasis_settitle(String(v));
        __sync_freed();
      }
    },
    readyState: { get: function() { return __ready_state; } },
    URL: { get: function() { return __oasis_location(); } },
    documentURI: { get: function() { return __oasis_location(); } },
    location: {
      get: function() { return globalThis.location; },
      set: function(v) { __oasis_location_assign(String(v)); }
    },
    defaultView: { get: function() { return globalThis; } },
    characterSet: { get: function() { return 'UTF-8'; } },
    compatMode: { get: function() { return 'CSS1Compat'; } },
    visibilityState: { get: function() { return 'visible'; } },
    hidden: { get: function() { return false; } }
  });

  // Page lifecycle, called by the host once parser-inserted scripts
  // have run: readystatechange(interactive) -> DOMContentLoaded
  // (bubbles to window) -> readystatechange(complete) -> window load.
  globalThis.__oasis_fire_lifecycle = function() {
    if (__ready_state !== 'loading') return;
    __ready_state = 'interactive';
    __dispatch(document, new Event('readystatechange'));
    __dispatch(document, new Event('DOMContentLoaded', {bubbles: true}));
    __ready_state = 'complete';
    __dispatch(document, new Event('readystatechange'));
    __dispatch(globalThis, new Event('load'));
  };

  globalThis.document = document;
  globalThis.Node = Node;
  globalThis.CharacterData = CharacterData;
  globalThis.Text = Text;
  globalThis.Comment = Comment;
  globalThis.Element = Element;
  globalThis.HTMLElement = Element;
  globalThis.DocumentFragment = DocumentFragment;
  globalThis.Document = Document;
  globalThis.HTMLDocument = Document;
  globalThis.DOMTokenList = DOMTokenList;
  globalThis.Event = Event;
  globalThis.CustomEvent = CustomEvent;
  globalThis.UIEvent = UIEvent;
  globalThis.MouseEvent = MouseEvent;
  globalThis.KeyboardEvent = KeyboardEvent;
  globalThis.FocusEvent = FocusEvent;
  globalThis.InputEvent = InputEvent;
  globalThis.window = globalThis;
  globalThis.self = globalThis;

  // -- window as an event target ------------------------------------------
  globalThis.addEventListener = function(type, fn, opts) {
    __add_listener(globalThis, String(type), fn, opts);
  };
  globalThis.removeEventListener = function(type, fn, opts) {
    __remove_listener(globalThis, String(type), fn, __parse_opts(opts).capture);
  };
  globalThis.dispatchEvent = function(evt) {
    return __dispatch(globalThis, __as_event(evt));
  };

  // -- requestAnimationFrame (timer-backed, ~60 Hz) ------------------------
  var __time_origin = Date.now();
  if (typeof globalThis.performance === 'undefined') {
    globalThis.performance = {
      timeOrigin: __time_origin,
      now: function() { return Date.now() - __time_origin; }
    };
  }
  if (typeof setTimeout === 'function') {
    globalThis.requestAnimationFrame = function(cb) {
      return setTimeout(function() { cb(performance.now()); }, 16);
    };
    globalThis.cancelAnimationFrame = function(id) { clearTimeout(id); };
  }

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

  // -- fetch API --
  // `fetch` / `Response` / `Headers` come from oasis-js (real Promises);
  // js_dom/fetch.rs binds the page's origin-aware transport behind them.

  // -- getComputedStyle --
  globalThis.getComputedStyle = function(el) {
    return {
      getPropertyValue: function(prop) {
        return __oasis_computed_style(el.__oasis_node_id, prop);
      }
    };
  };

  // -- localStorage / sessionStorage --
  // kind: 0 = localStorage, 1 = sessionStorage (separate, per-origin
  // backing stores). Missing keys come back as null; "" is a real value.
  var __make_storage = function(kind) {
    return {
      getItem: function(k) {
        var v = __oasis_storage_get(kind, String(k));
        return v === undefined ? null : v;
      },
      setItem: function(k, v) {
        if (!__oasis_storage_set(kind, String(k), String(v))) {
          throw new DOMException(
            "Failed to execute 'setItem' on 'Storage': quota exceeded",
            'QuotaExceededError');
        }
      },
      removeItem: function(k) { __oasis_storage_remove(kind, String(k)); },
      clear: function() { __oasis_storage_clear(kind); },
      key: function(i) {
        var k = __oasis_storage_key(kind, Number(i) | 0);
        return k === undefined ? null : k;
      },
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
