// ══════════════════════════════════════════════════════════
//  BLUEPRINT ENGINE — reusable 3D viewer for hardware diagrams
//  Expects: THREE.js loaded, DIAGRAMS registered via BlueprintEngine.register()
// ══════════════════════════════════════════════════════════

var BlueprintEngine = (function(){

// ── Drawing Helpers ──────────────────────────────────────
function lm(c){ return new THREE.LineBasicMaterial({color:c}); }
function fm(c,op){ return new THREE.MeshBasicMaterial({color:c,transparent:true,opacity:op||0.35,side:THREE.DoubleSide}); }
function v(x,y,z){ return new THREE.Vector3(x,y,z); }

function wireBox(w,h,d,lineMat,fillMat){
  var g = new THREE.Group();
  var geo = new THREE.BoxGeometry(w,h,d);
  g.add(new THREE.LineSegments(new THREE.EdgesGeometry(geo), lineMat));
  if(fillMat) g.add(new THREE.Mesh(geo, fillMat));
  return g;
}

function cyl(r,h,segs,material){
  var geo = new THREE.CylinderGeometry(r,r,h,segs);
  return new THREE.LineSegments(new THREE.EdgesGeometry(geo), material);
}

function plane(w,h,mat){
  return new THREE.Mesh(new THREE.PlaneGeometry(w,h), mat);
}

function ring(inner,outer,segs,mat){
  return new THREE.LineSegments(new THREE.EdgesGeometry(new THREE.RingGeometry(inner,outer,segs)), mat);
}

function circle(r,segs,mat){
  return new THREE.LineSegments(new THREE.EdgesGeometry(new THREE.CircleGeometry(r,segs)), mat);
}

function sphere(r,segs,mat){
  return new THREE.Mesh(new THREE.SphereGeometry(r,segs,segs), mat);
}

function cable(pts, material){
  var curve = new THREE.CatmullRomCurve3(pts, false, 'catmullrom', 0.35);
  var geo = new THREE.BufferGeometry().setFromPoints(curve.getPoints(50));
  return new THREE.Line(geo, material);
}

function tag(text, pos, color, target){
  var g = new THREE.Group();

  // Fixed 3D text — canvas sized to fit, centered
  var measure = document.createElement('canvas').getContext('2d');
  measure.font = 'bold 22px Courier New';
  var textW = Math.ceil(measure.measureText(text).width) + 16;
  var canW = Math.max(textW, 64);
  var canH = 40;

  var c = document.createElement('canvas'); c.width=canW; c.height=canH;
  var ctx = c.getContext('2d');
  ctx.font = 'bold 22px Courier New';
  ctx.fillStyle = color || '#4488cc';
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  ctx.fillText(text, canW/2, canH/2);

  var tex = new THREE.CanvasTexture(c);
  tex.minFilter = THREE.LinearFilter;
  var mat = new THREE.MeshBasicMaterial({map:tex, transparent:true, depthTest:false, side:THREE.DoubleSide});
  var planeW = canW * 0.013;
  var planeH = canH * 0.013;
  var mesh = new THREE.Mesh(new THREE.PlaneGeometry(planeW, planeH), mat);
  mesh.position.copy(pos);
  g.add(mesh);

  if(target){
    var lineGeo = new THREE.BufferGeometry().setFromPoints([pos, target]);
    var lineCol = parseInt((color||'#4488cc').replace('#',''),16);
    g.add(new THREE.Line(lineGeo, new THREE.LineBasicMaterial({color:lineCol, transparent:true, opacity:0.4})));
    var dot = new THREE.Mesh(new THREE.SphereGeometry(0.12,6,4), new THREE.MeshBasicMaterial({color:lineCol}));
    dot.position.copy(target);
    g.add(dot);
  }
  return g;
}

// Fixed 3D text label — stays in world space, doesn't billboard.
// Faces +Z by default (toward camera in front view). Optional rotation.
function tag3D(text, pos, color, rotX, rotY){
  // Size canvas to fit text tightly
  var measure = document.createElement('canvas').getContext('2d');
  measure.font = 'bold 28px Courier New';
  var textW = Math.ceil(measure.measureText(text).width) + 16;
  var canW = Math.max(textW, 64);
  var canH = 48;

  var c = document.createElement('canvas'); c.width=canW; c.height=canH;
  var ctx = c.getContext('2d');
  ctx.font = 'bold 28px Courier New';
  ctx.fillStyle = color || '#4488cc';
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  ctx.fillText(text, canW/2, canH/2);

  var tex = new THREE.CanvasTexture(c);
  tex.minFilter = THREE.LinearFilter;
  var mat = new THREE.MeshBasicMaterial({map:tex, transparent:true, depthTest:false, side:THREE.DoubleSide});
  // Scale: ~0.013 world units per canvas pixel
  var planeW = canW * 0.013;
  var planeH = canH * 0.013;
  var mesh = new THREE.Mesh(new THREE.PlaneGeometry(planeW, planeH), mat);
  mesh.position.copy(pos);
  if(rotX) mesh.rotation.x = rotX;
  if(rotY) mesh.rotation.y = rotY;
  return mesh;
}

function dimLine(p1,p2,label,col){
  var g = new THREE.Group();
  g.add(new THREE.Line(new THREE.BufferGeometry().setFromPoints([p1,p2]), lm(col||0x1e3a55)));
  var mid = p1.clone().add(p2).multiplyScalar(0.5); mid.y += 0.4;
  g.add(tag(label, mid, col?('#'+col.toString(16).padStart(6,'0')):'#2a5588'));
  return g;
}

var helpers = {
  lm:lm, fm:fm, v:v,
  wireBox:wireBox, cyl:cyl, plane:plane, ring:ring, circle:circle, sphere:sphere,
  cable:cable, tag:tag, tag3D:tag3D, dimLine:dimLine
};


// ── Shared Components ────────────────────────────────────

// Cached PSP GLB model (loaded async before diagram build)
var _pspModel = null; // THREE.Group or null
var _pspModelHeight = 7.14; // updated from model bbox after load

// Preload PSP model from GLB. Returns a Promise.
function preloadPSPModel(){
  return new Promise(function(resolve){
    if(_pspModel){ resolve(_pspModel); return; }
    if(typeof THREE.GLTFLoader === 'undefined'){ resolve(null); return; }
    var loader = new THREE.GLTFLoader();
    loader.load('models/psp.glb', function(gltf){
      var scene = gltf.scene;
      // Orient: FBX model lies flat. Stand upright with screen facing +Z, d-pad on left.
      // PI/2-0.75 compensates for the model's built-in tilt from the FBX export.
      scene.rotation.set(Math.PI/2 - 0.75, 0, 0);
      // Wrap in pivot for clean transform
      var pivot = new THREE.Group();
      pivot.add(scene);
      // Scale to match unit system (1 unit = 10mm, PSP ~17 units wide)
      pivot.updateMatrixWorld(true);
      var box = new THREE.Box3().setFromObject(pivot);
      var rawWidth = box.max.x - box.min.x;
      var scale = 17 / rawWidth;
      pivot.scale.set(scale, scale, scale);
      // Shift so bottom sits at Y=0
      pivot.updateMatrixWorld(true);
      var box2 = new THREE.Box3().setFromObject(pivot);
      pivot.position.y = -box2.min.y;
      // Measure actual dimensions and center on Z axis
      var finalBox = new THREE.Box3().setFromObject(pivot);
      _pspModelHeight = finalBox.max.y;
      // Center Z so model sits at Z=0 (adapter components are at Z=0)
      var zCenter = (finalBox.min.z + finalBox.max.z) / 2;
      pivot.position.z = -zCenter;
      _pspModel = pivot;
      resolve(pivot);
    }, undefined, function(){
      resolve(null); // fallback to procedural
    });
  });
}

// Creates a PSP-3001 body (standing upright, screen facing +Z).
// Uses GLB model with wireframe + dark fill if available, falls back to procedural.
// Unit system: 1 unit = 10mm.
// PSP-3001 specs: 169.4 × 71.4 × 18.6mm (Sony product specifications)
// Returns { topY, pH, pW, pD, faceZ }
function buildPSP(parentGroup){
  var pW = 17, pD = 1.86;
  var faceZ = pD/2 + 0.02;
  var G = parentGroup;
  // pH derived from model if available, otherwise fallback
  var pH = _pspModel ? _pspModelHeight : 7.14;

  if(_pspModel){
    // Clone the cached model and apply wireframe + dark fill style
    var clone = _pspModel.clone(true);
    clone.traverse(function(child){
      if(child.isMesh){
        // Semi-transparent dark fill
        child.material = fm(0x0e0e18, 0.25);
        // Add edge wireframe as sibling
        var edges = new THREE.EdgesGeometry(child.geometry, 30);
        var line = new THREE.LineSegments(edges, lm(0x4488ff));
        line.position.copy(child.position);
        line.rotation.copy(child.rotation);
        line.scale.copy(child.scale);
        child.parent.add(line);
      }
    });
    G.add(clone);
  } else {
    // Procedural fallback (no model loaded)
    buildPSPProcedural(G, pW, pH, pD, faceZ);
  }

  // Top-edge details (always added on top of either model or procedural)
  // Layout matches real PSP-3001 top edge (from photos):
  //   [screw]  [5V pad] [GND pad]  [Mini-B USB]  [GND pad] [5V pad]  [screw]
  //   x=-2.0   x=-1.1   x=-0.7     x=0           x=0.7     x=1.1     x=2.0

  // Mini-B port (~8mm wide)
  var miniB = wireBox(0.8, 0.5, 0.6, lm(0xcc44ff));
  miniB.position.set(0, pH+0.25, 0);
  G.add(miniB);

  // Power pads (one 5V pad on each side of USB port)
  [-1.0, 1.0].forEach(function(x){
    var pad = wireBox(0.3, 0.15, 0.4, lm(0xffaa00), fm(0xffaa00,0.6));
    pad.position.set(x, pH+0.1, 0);
    G.add(pad);
    G.add(tag('5V', v(x, pH+0.7, 0), '#ffaa00'));
  });

  // Screw holes (far edges of top panel)
  [-2.0, 2.0].forEach(function(x){
    var hole = ring(0.08, 0.15, 8, lm(0x888888));
    hole.position.set(x, pH+0.15, 0);
    G.add(hole);
  });

  G.add(tag('Mini-B USB', v(0, pH-0.5, -pD/2-1.5), '#cc44ff', v(0, pH+0.25, 0)));

  return { topY: pH, pH: pH, pW: pW, pD: pD, faceZ: faceZ };
}

// Procedural PSP fallback (used when GLB model isn't available)
function buildPSPProcedural(G, pW, pH, pD, faceZ){
  var pCY = pH/2;

  var body = wireBox(pW, pH, pD, lm(0x333344), fm(0x0e0e18,0.3));
  body.position.set(0, pCY, 0); G.add(body);
  var gripL = wireBox(3.5, pH-1, pD+0.4, lm(0x2a2a3a), fm(0x0e0e18,0.15));
  gripL.position.set(-7.5, pCY-0.3, 0); G.add(gripL);
  var gripR = wireBox(3.5, pH-1, pD+0.4, lm(0x2a2a3a), fm(0x0e0e18,0.15));
  gripR.position.set(7.5, pCY-0.3, 0); G.add(gripR);

  var bezel = wireBox(11, 5.2, 0.15, lm(0x222233), fm(0x060610,0.4));
  bezel.position.set(0, pCY+0.2, faceZ); G.add(bezel);
  var scrMesh = plane(9.5, 4.5, fm(0x112844, 0.5));
  scrMesh.position.set(0, pCY+0.2, faceZ+0.02); G.add(scrMesh);

  var dpad = ring(0.6, 0.8, 16, lm(0x444455));
  dpad.position.set(-7, pCY, faceZ+0.02); G.add(dpad);
  [[0,0.6,0x55aa77],[0.6,0,0xff5566],[0,-0.6,0x5566cc],[-0.6,0,0xcc55aa]].forEach(function(b){
    var btn = ring(0.2, 0.3, 10, lm(b[2]));
    btn.position.set(7+b[0], pCY+b[1], faceZ+0.02); G.add(btn);
  });
  var nub = circle(0.4, 12, lm(0x555566));
  nub.position.set(-5, pCY-2, faceZ+0.02); G.add(nub);

  var shL = wireBox(2.5, 0.5, pD-0.3, lm(0x444455));
  shL.position.set(-6.5, pH, 0); G.add(shL);
  var shR = wireBox(2.5, 0.5, pD-0.3, lm(0x444455));
  shR.position.set(6.5, pH, 0); G.add(shR);

  G.add(tag('SONY', v(7, pCY-2.5, faceZ+0.1), '#333344'));
  [['HOME',-6.5],['VOL',-4],['PSP',0],['SELECT',4],['START',6.5]].forEach(function(l){
    G.add(tag(l[0], v(l[1], 0.3, faceZ+0.1), '#334455'));
  });
}

// ── Orbit Controls ───────────────────────────────────────
function createOrbitControls(camera, renderer, camConfig){
  var orbit = {
    drag:false, px:0, py:0, mode:null,
    r:camConfig.r, phi:camConfig.phi, theta:camConfig.theta,
    target: new THREE.Vector3(camConfig.target[0], camConfig.target[1], camConfig.target[2])
  };

  function update(){
    orbit.phi = Math.max(0.1, Math.min(Math.PI-0.1, orbit.phi));
    orbit.r = Math.max(5, Math.min(90, orbit.r));
    camera.position.set(
      orbit.target.x + orbit.r*Math.sin(orbit.phi)*Math.sin(orbit.theta),
      orbit.target.y + orbit.r*Math.cos(orbit.phi),
      orbit.target.z + orbit.r*Math.sin(orbit.phi)*Math.cos(orbit.theta)
    );
    camera.lookAt(orbit.target);
  }
  update();

  var el = renderer.domElement;
  el.addEventListener('pointerdown',function(e){
    orbit.px=e.clientX; orbit.py=e.clientY;
    if(e.button===0) orbit.mode='rotate';
    else if(e.button===1){orbit.mode='pan'; e.preventDefault();}
    orbit.drag=true;
  });
  el.addEventListener('contextmenu',function(e){e.preventDefault();});
  window.addEventListener('pointerup',function(){orbit.drag=false; orbit.mode=null;});
  window.addEventListener('pointermove',function(e){
    if(!orbit.drag) return;
    var dx=e.clientX-orbit.px, dy=e.clientY-orbit.py;
    orbit.px=e.clientX; orbit.py=e.clientY;
    if(orbit.mode==='rotate'){orbit.theta-=dx*0.008; orbit.phi-=dy*0.008;}
    else if(orbit.mode==='pan'){
      var ps=orbit.r*0.002, fw=new THREE.Vector3();
      camera.getWorldDirection(fw);
      var rt=new THREE.Vector3().crossVectors(fw,camera.up).normalize();
      var up=new THREE.Vector3().crossVectors(rt,fw).normalize();
      orbit.target.add(rt.multiplyScalar(-dx*ps));
      orbit.target.add(up.multiplyScalar(dy*ps));
    }
    update();
  });
  el.addEventListener('wheel',function(e){orbit.r+=e.deltaY*0.03;update();},{passive:true});
  el.addEventListener('mousedown',function(e){if(e.button===1)e.preventDefault();});

  // Expose globally for screenshot script
  window.orbit = orbit;
  window.camUpdate = update;

  return {orbit:orbit, update:update};
}

// ── Explode Controller ───────────────────────────────────
function createExplodeController(groups, explodeSmallY, explodeGrid){
  var origPos = {};
  Object.keys(groups).forEach(function(k){
    origPos[k] = {x:groups[k].position.x, y:groups[k].position.y, z:groups[k].position.z};
  });

  function apply(t){
    var f=t/100, f1=Math.min(f*2,1), f2=Math.max((f-0.5)*2,0);
    Object.keys(groups).forEach(function(k){
      // Dims stay fixed as measurement references during explode
      if(k==='dims') return;
      var oy=(explodeSmallY[k]||0)*f1;
      var grid=explodeGrid[k]||{x:0,y:0,z:0};
      var orig=origPos[k];
      groups[k].position.x = orig.x + grid.x*f2;
      groups[k].position.y = orig.y + oy + grid.y*f2;
      groups[k].position.z = orig.z + grid.z*f2;
    });
  }

  var slider = document.getElementById('explode');
  slider.addEventListener('input',function(){apply(parseFloat(slider.value));});
  apply(parseFloat(slider.value));

  return {apply:apply, slider:slider};
}

// ── Hover Tooltips ───────────────────────────────────────
function createHoverSystem(camera, renderer, hoverData){
  if(!hoverData || !hoverData.length) return;
  var tooltipEl = document.getElementById('tooltip');
  var raycaster = new THREE.Raycaster();
  var mouse = new THREE.Vector2();
  var meshMap = {};

  hoverData.forEach(function(h,i){
    var m = [];
    h.group.traverse(function(c){if(c.isMesh||c.isLineSegments||c.isLine) m.push(c);});
    meshMap[i] = m;
  });

  renderer.domElement.addEventListener('pointermove',function(e){
    if(window.orbit && window.orbit.drag){tooltipEl.style.display='none';return;}
    mouse.x=(e.clientX/innerWidth)*2-1;
    mouse.y=-(e.clientY/innerHeight)*2+1;
    raycaster.setFromCamera(mouse, camera);
    var found=null;
    for(var i=0; i<hoverData.length; i++){
      if(raycaster.intersectObjects(meshMap[i],false).length>0){found=hoverData[i]; break;}
    }
    if(found){
      tooltipEl.style.display='block';
      tooltipEl.style.left=Math.min(e.clientX+16,innerWidth-340)+'px';
      tooltipEl.style.top=Math.min(e.clientY+16,innerHeight-160)+'px';
      tooltipEl.innerHTML='<div class="label">'+found.label+'</div>'+found.desc;
      document.body.style.cursor='pointer';
    } else {
      tooltipEl.style.display='none';
      document.body.style.cursor='default';
    }
  });
}

// ── Tab System (data-driven) ─────────────────────────────
function createTabs(tabs, tabStates, explodeCtrl){
  if(!tabs || !tabs.length) return;
  var tabsEl = document.getElementById('tabs');
  var activeId = tabs[0].id;

  function activateTab(id){
    activeId = id;
    tabsEl.querySelectorAll('button').forEach(function(b){
      b.classList.toggle('active', b.dataset.tabId===id);
    });
    var state = tabStates && tabStates[id];
    if(state){
      if(state.note){
        document.getElementById('state-note').innerHTML = state.note.replace(/\n/g,'<br>');
      }
      // Data-driven explode: diagram declares the slider value per tab
      if(typeof state.explode === 'number'){
        explodeCtrl.slider.value = state.explode;
        explodeCtrl.apply(state.explode);
      }
    }
  }

  tabs.forEach(function(tab, i){
    var btn = document.createElement('button');
    btn.textContent = tab.label;
    btn.dataset.tabId = tab.id;
    if(i===0) btn.classList.add('active');
    btn.addEventListener('click', function(){ activateTab(tab.id); });
    tabsEl.appendChild(btn);
  });

  // Apply initial tab state
  activateTab(activeId);
}

// ── HUD Population ───────────────────────────────────────
function populateHUD(diagram){
  document.getElementById('title').textContent = diagram.title;
  document.getElementById('subtitle').innerHTML = diagram.subtitle;
  if(diagram.dims) document.getElementById('dims').innerHTML = diagram.dims.replace(/\n/g,'<br>');
  if(diagram.backLink){
    document.getElementById('back-link').innerHTML = '<a href="'+diagram.backLink.href+'">'+diagram.backLink.text+'</a>';
  }
  if(diagram.legend){
    document.getElementById('legend').innerHTML = diagram.legend.map(function(l){
      return '<div class="row"><span class="swatch" style="background:'+l.color+'"></span> '+l.label+'</div>';
    }).join('');
  }
  if(diagram.stateNotes){
    document.getElementById('state-note').innerHTML = diagram.stateNotes.join('<br>');
  }
}

// ── Scene Lifecycle ──────────────────────────────────────
var _canvas = null;
var _resizeHandler = null;

function cleanup(){
  // Remove old canvas from DOM
  if(_canvas && _canvas.parentNode){
    _canvas.parentNode.removeChild(_canvas);
    _canvas = null;
  }
  // Remove old resize listener
  if(_resizeHandler){
    window.removeEventListener('resize', _resizeHandler);
    _resizeHandler = null;
  }
  // Clear tabs
  document.getElementById('tabs').innerHTML = '';
  // Reset HUD
  document.getElementById('state-note').innerHTML = '';
  document.getElementById('tooltip').style.display = 'none';
}

// ── Main Init ────────────────────────────────────────────
var diagrams = {};

function register(id, config){ diagrams[id] = config; }

function init(){
  cleanup();

  var params = new URLSearchParams(window.location.search);
  var id = params.get('diagram') || Object.keys(diagrams)[0];
  var diagram = diagrams[id];
  if(!diagram){
    document.getElementById('loading').style.display='flex';
    document.getElementById('loading').textContent='UNKNOWN DIAGRAM: '+id;
    return;
  }

  // Preload PSP model, then build diagram
  preloadPSPModel().then(function(){
    document.getElementById('loading').style.display = 'none';
    initDiagram(diagram);
  });
}

function initDiagram(diagram){
  populateHUD(diagram);

  // Scene
  var scene = new THREE.Scene();
  scene.fog = new THREE.FogExp2(0x0a1628, 0.008);
  scene.add(new THREE.GridHelper(50, 50, 0x152540, 0x0e1a2e));

  var cam = diagram.camera || {r:35, phi:Math.PI/3.5, theta:Math.PI/5, target:[0,5,0]};
  var camera = new THREE.PerspectiveCamera(40, innerWidth/innerHeight, 0.1, 500);
  var renderer = new THREE.WebGLRenderer({antialias:true});
  renderer.setSize(innerWidth, innerHeight);
  renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
  renderer.setClearColor(0x0a1628);
  _canvas = renderer.domElement;
  document.body.appendChild(_canvas);

  createOrbitControls(camera, renderer, cam);

  // Build diagram with error handling
  var result;
  try {
    result = diagram.build(scene, helpers);
  } catch(e) {
    document.getElementById('loading').style.display='flex';
    document.getElementById('loading').textContent='BUILD ERROR: '+e.message;
    return;
  }

  // Provide defaults for missing return fields
  result.groups = result.groups || {};
  result.explodeSmallY = result.explodeSmallY || {};
  result.explodeGrid = result.explodeGrid || {};
  result.hoverData = result.hoverData || [];
  result.tabStates = result.tabStates || {};

  var explodeCtrl = createExplodeController(result.groups, result.explodeSmallY, result.explodeGrid);
  createHoverSystem(camera, renderer, result.hoverData);
  createTabs(diagram.tabs, result.tabStates, explodeCtrl);

  // Render loop
  (function animate(){
    requestAnimationFrame(animate);
    renderer.render(scene, camera);
  })();

  _resizeHandler = function(){
    camera.aspect=innerWidth/innerHeight;
    camera.updateProjectionMatrix();
    renderer.setSize(innerWidth, innerHeight);
  };
  window.addEventListener('resize', _resizeHandler);
}

// Public API
return {
  register: register,
  init: init,
  helpers: helpers,
  buildPSP: buildPSP
};

})();

// Auto-init on load
window.addEventListener('load', function(){ BlueprintEngine.init(); });
