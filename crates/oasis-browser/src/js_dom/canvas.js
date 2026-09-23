(function() {
  "use strict";

  function CanvasRenderingContext2D(nid) {
    this.__nid = nid;
    this._fillStyle = "#000000";
    this._strokeStyle = "#000000";
    this._lineWidth = 1;
    this._font = "10px sans-serif";
    this._pathX = 0;
    this._pathY = 0;
    this._pathSegments = [];
  }

  Object.defineProperties(CanvasRenderingContext2D.prototype, {
    fillStyle: {
      get: function() { return this._fillStyle; },
      set: function(v) {
        this._fillStyle = v;
        __oasis_canvas_set_fill(this.__nid, String(v));
      },
      enumerable: true
    },
    strokeStyle: {
      get: function() { return this._strokeStyle; },
      set: function(v) {
        this._strokeStyle = v;
        __oasis_canvas_set_stroke(this.__nid, String(v));
      },
      enumerable: true
    },
    lineWidth: {
      get: function() { return this._lineWidth; },
      set: function(v) {
        this._lineWidth = v;
        __oasis_canvas_set_line_width(this.__nid, +v);
      },
      enumerable: true
    },
    font: {
      get: function() { return this._font; },
      set: function(v) {
        this._font = v;
        __oasis_canvas_set_font(this.__nid, String(v));
      },
      enumerable: true
    }
  });

  CanvasRenderingContext2D.prototype.fillRect = function(x, y, w, h) {
    __oasis_canvas_fill_rect(this.__nid, +x, +y, +w, +h);
  };
  CanvasRenderingContext2D.prototype.strokeRect = function(x, y, w, h) {
    __oasis_canvas_stroke_rect(this.__nid, +x, +y, +w, +h);
  };
  CanvasRenderingContext2D.prototype.clearRect = function(x, y, w, h) {
    __oasis_canvas_clear_rect(this.__nid, +x, +y, +w, +h);
  };
  CanvasRenderingContext2D.prototype.fillText = function(text, x, y) {
    __oasis_canvas_fill_text(this.__nid, String(text), +x, +y);
  };
  CanvasRenderingContext2D.prototype.strokeText = function() {};
  CanvasRenderingContext2D.prototype.beginPath = function() {
    __oasis_canvas_begin_path(this.__nid);
  };
  CanvasRenderingContext2D.prototype.moveTo = function(x, y) {
    __oasis_canvas_move_to(this.__nid, +x, +y);
    this._pathX = +x;
    this._pathY = +y;
  };
  CanvasRenderingContext2D.prototype.lineTo = function(x, y) {
    __oasis_canvas_line_to(this.__nid, +x, +y);
    this._pathX = +x;
    this._pathY = +y;
  };
  CanvasRenderingContext2D.prototype.bezierCurveTo = function(cp1x, cp1y, cp2x, cp2y, x, y) {
    __oasis_canvas_bezier_curve_to(this.__nid, +cp1x, +cp1y, +cp2x, +cp2y, +x, +y);
    this._pathX = +x;
    this._pathY = +y;
  };
  CanvasRenderingContext2D.prototype.quadraticCurveTo = function(cpx, cpy, x, y) {
    __oasis_canvas_quadratic_curve_to(this.__nid, +cpx, +cpy, +x, +y);
    this._pathX = +x;
    this._pathY = +y;
  };
  CanvasRenderingContext2D.prototype.arc = function(cx, cy, r) {
    // Arc is handled specially: emit as native arc command.
    this._pathSegments.push({
      type: "arc", cx: +cx, cy: +cy, r: +r
    });
  };
  CanvasRenderingContext2D.prototype.closePath = function() {
    __oasis_canvas_close_path(this.__nid);
  };
  CanvasRenderingContext2D.prototype.fill = function() {
    // First flush any arc segments (legacy path).
    for (var i = 0; i < this._pathSegments.length; i++) {
      var seg = this._pathSegments[i];
      if (seg.type === "arc") {
        __oasis_canvas_arc(this.__nid, seg.cx, seg.cy, seg.r, true);
      }
    }
    this._pathSegments = [];
    // Then emit the native path fill.
    __oasis_canvas_fill_path(this.__nid);
  };
  CanvasRenderingContext2D.prototype.stroke = function() {
    // First flush any arc segments (legacy path).
    for (var i = 0; i < this._pathSegments.length; i++) {
      var seg = this._pathSegments[i];
      if (seg.type === "arc") {
        __oasis_canvas_arc(this.__nid, seg.cx, seg.cy, seg.r, false);
      }
    }
    this._pathSegments = [];
    // Then emit the native path stroke.
    __oasis_canvas_stroke_path(this.__nid);
  };
  CanvasRenderingContext2D.prototype.measureText = function(text) {
    return { width: String(text).length * 6 };
  };
  CanvasRenderingContext2D.prototype.save = function() {
    __oasis_canvas_save(this.__nid);
  };
  CanvasRenderingContext2D.prototype.restore = function() {
    __oasis_canvas_restore(this.__nid);
  };

  var __canvas_contexts = {};

  if (typeof Element !== "undefined") {
    Element.prototype.getContext = function(type) {
      if (type !== "2d") return null;
      var nid = this.__oasis_node_id;
      if (!__canvas_contexts[nid]) {
        __canvas_contexts[nid] = new CanvasRenderingContext2D(nid);
      }
      return __canvas_contexts[nid];
    };
  }

  globalThis.CanvasRenderingContext2D = CanvasRenderingContext2D;
})();
