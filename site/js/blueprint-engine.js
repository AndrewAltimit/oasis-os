// ══════════════════════════════════════════════════════════
//  BLUEPRINT ENGINE — reusable 3D viewer for hardware diagrams
//  Expects: THREE.js loaded, DIAGRAMS registered via BlueprintEngine.register()
// ══════════════════════════════════════════════════════════

var BlueprintEngine = (function(){

// ── Drawing Helpers ──────────────────────────────────────
function lm(c){ return new THREE.LineBasicMaterial({color:c}); }
function fm(c,op){ return new THREE.MeshBasicMaterial({color:c,transparent:true,opacity:op||0.35,side:THREE.DoubleSide}); }

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

function cable(pts, material){
  var curve = new THREE.CatmullRomCurve3(pts, false, 'catmullrom', 0.35);
  var geo = new THREE.BufferGeometry().setFromPoints(curve.getPoints(50));
  return new THREE.Line(geo, material);
}

function tag(text, pos, color, target){
  var g = new THREE.Group();
  var c = document.createElement('canvas'); c.width=512; c.height=64;
  var ctx = c.getContext('2d');
  ctx.font = 'bold 22px Courier New';
  ctx.fillStyle = color || '#4488cc';
  ctx.fillText(text, 6, 36);
  var tex = new THREE.CanvasTexture(c);
  tex.minFilter = THREE.LinearFilter;
  var sp = new THREE.Sprite(new THREE.SpriteMaterial({map:tex, transparent:true, depthTest:false}));
  sp.scale.set(6.5, 0.85, 1);
  sp.position.copy(pos);
  g.add(sp);
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

function dimLine(p1,p2,label,col){
  var g = new THREE.Group();
  g.add(new THREE.Line(new THREE.BufferGeometry().setFromPoints([p1,p2]), lm(col||0x1e3a55)));
  var mid = p1.clone().add(p2).multiplyScalar(0.5); mid.y += 0.4;
  g.add(tag(label, mid, col?('#'+col.toString(16).padStart(6,'0')):'#2a5588'));
  return g;
}

var helpers = { lm:lm, fm:fm, wireBox:wireBox, cyl:cyl, cable:cable, tag:tag, dimLine:dimLine };


// ── Shared Components ────────────────────────────────────

// Creates a detailed PSP-3001 body (standing upright, screen facing +Z)
// Returns { group, topY, pH, pW, pD, faceZ }
function buildPSP(parentGroup){
  var pW = 17, pH = 7.14, pD = 1.86;
  var pCY = pH/2;
  var faceZ = pD/2 + 0.02;
  var G = parentGroup;

  // Main body
  var body = wireBox(pW, pH, pD, lm(0x333344), fm(0x0e0e18,0.3));
  body.position.set(0, pCY, 0);
  G.add(body);

  // Grips
  var gripL = wireBox(3.5, pH-1, pD+0.4, lm(0x2a2a3a), fm(0x0e0e18,0.15));
  gripL.position.set(-7.5, pCY-0.3, 0);
  G.add(gripL);
  var gripR = wireBox(3.5, pH-1, pD+0.4, lm(0x2a2a3a), fm(0x0e0e18,0.15));
  gripR.position.set(7.5, pCY-0.3, 0);
  G.add(gripR);

  // Screen bezel
  var bezel = wireBox(11, 5.2, 0.15, lm(0x222233), fm(0x060610,0.4));
  bezel.position.set(0, pCY+0.2, faceZ);
  G.add(bezel);

  // LCD screen
  var scrGeo = new THREE.PlaneGeometry(9.5, 4.5);
  var scrMesh = new THREE.Mesh(scrGeo, fm(0x112844, 0.5));
  scrMesh.position.set(0, pCY+0.2, faceZ+0.02);
  G.add(scrMesh);
  var scrBorder = new THREE.LineSegments(new THREE.EdgesGeometry(scrGeo), lm(0x224466));
  scrBorder.position.copy(scrMesh.position);
  G.add(scrBorder);

  // Scan lines
  for(var sl=0; sl<8; sl++){
    var scan = new THREE.Mesh(new THREE.PlaneGeometry(9.3, 0.02), fm(0x44aaff, 0.08));
    scan.position.set(0, pCY+0.2-4.5/2+0.3+sl*(4.5/8), faceZ+0.03);
    G.add(scan);
  }

  // D-pad
  var dpad = new THREE.LineSegments(new THREE.EdgesGeometry(new THREE.RingGeometry(0.6, 0.8, 16)), lm(0x444455));
  dpad.position.set(-7, pCY, faceZ+0.02);
  G.add(dpad);
  [[0,0.55,0,-0.55],[0.55,0,-0.55,0]].forEach(function(c){
    G.add(new THREE.Line(new THREE.BufferGeometry().setFromPoints([
      new THREE.Vector3(-7+c[0], pCY+c[1], faceZ+0.03),
      new THREE.Vector3(-7+c[2], pCY+c[3], faceZ+0.03)
    ]), lm(0x444455)));
  });

  // Face buttons
  [[0,0.6,0x55aa77],[0.6,0,0xff5566],[0,-0.6,0x5566cc],[-0.6,0,0xcc55aa]].forEach(function(b){
    var btn = new THREE.LineSegments(new THREE.EdgesGeometry(new THREE.RingGeometry(0.2, 0.3, 10)), lm(b[2]));
    btn.position.set(7+b[0], pCY+b[1], faceZ+0.02);
    G.add(btn);
  });

  // Analog nub
  var nub = new THREE.LineSegments(new THREE.EdgesGeometry(new THREE.CircleGeometry(0.4, 12)), lm(0x555566));
  nub.position.set(-5, pCY-2, faceZ+0.02);
  G.add(nub);

  // Speaker grilles
  [-5.8, 5.8].forEach(function(x){
    for(var r=0; r<3; r++){
      for(var c=0; c<2; c++){
        var dot = new THREE.Mesh(new THREE.CircleGeometry(0.06, 6), fm(0x222233, 0.6));
        dot.position.set(x+c*0.25, pCY+0.8-r*0.25, faceZ+0.02);
        G.add(dot);
      }
    }
  });

  // Shoulder buttons
  var shL = wireBox(2.5, 0.5, pD-0.3, lm(0x444455));
  shL.position.set(-6.5, pH, 0);
  G.add(shL);
  var shR = wireBox(2.5, 0.5, pD-0.3, lm(0x444455));
  shR.position.set(6.5, pH, 0);
  G.add(shR);

  // Power slider
  var pwr = wireBox(1.5, 0.3, 0.4, lm(0x556655));
  pwr.position.set(-8.2, pH, 0);
  G.add(pwr);

  // SONY logo
  G.add(tag('SONY', new THREE.Vector3(7, pCY-2.5, faceZ+0.1), '#333344'));

  // Bottom bar
  [['HOME',-6.5],['VOL',-4],['PSP',0],['SELECT',4],['START',6.5]].forEach(function(l){
    G.add(tag(l[0], new THREE.Vector3(l[1], 0.3, faceZ+0.1), '#334455'));
  });

  // Mini-B port on top edge
  var miniB = wireBox(1.2, 0.6, 0.8, lm(0xcc44ff));
  miniB.position.set(0, pH+0.3, 0);
  G.add(miniB);

  // Power pads on top edge
  [{x:-4,c:0xffaa00,l:'5V'},{x:-2.5,c:0x888888,l:'GND'},{x:2.5,c:0x888888,l:'GND'},{x:4,c:0xffaa00,l:'5V'}].forEach(function(p){
    var pad = wireBox(0.6, 0.2, 0.5, lm(p.c), fm(p.c,0.5));
    pad.position.set(p.x, pH+0.1, 0);
    G.add(pad);
    G.add(tag(p.l, new THREE.Vector3(p.x, pH+0.8, 0), p.c===0xffaa00?'#ffaa00':'#888888'));
  });

  G.add(tag('Mini-B USB', new THREE.Vector3(0, pH-0.5, -pD/2-1.5), '#cc44ff', new THREE.Vector3(0, pH+0.3, 0)));

  return { topY: pH, pH: pH, pW: pW, pD: pD, faceZ: faceZ };
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
  if(!hoverData) return;
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

// ── Tab System ───────────────────────────────────────────
function createTabs(tabs, tabStates, explodeCtrl){
  if(!tabs || !tabs.length) return;
  var tabsEl = document.getElementById('tabs');
  var activeId = tabs[0].id;

  tabs.forEach(function(tab, i){
    var btn = document.createElement('button');
    btn.textContent = tab.label;
    if(i===0) btn.classList.add('active');
    btn.addEventListener('click', function(){
      activeId = tab.id;
      tabsEl.querySelectorAll('button').forEach(function(b){b.classList.remove('active');});
      btn.classList.add('active');
      if(tabStates && tabStates[tab.id] && tabStates[tab.id].note){
        document.getElementById('state-note').innerHTML = tabStates[tab.id].note.replace(/\n/g,'<br>');
      }
      if(tab.id==='exploded'){explodeCtrl.slider.value=80; explodeCtrl.apply(80);}
      else if(tab.id==='assembled'){explodeCtrl.slider.value=20; explodeCtrl.apply(20);}
    });
    tabsEl.appendChild(btn);
  });

  if(tabStates && tabStates[activeId] && tabStates[activeId].note){
    document.getElementById('state-note').innerHTML = tabStates[activeId].note.replace(/\n/g,'<br>');
  }
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

// ── Main Init ────────────────────────────────────────────
var diagrams = {};

function register(id, config){ diagrams[id] = config; }

function init(){
  document.getElementById('loading').style.display = 'none';

  var params = new URLSearchParams(window.location.search);
  var id = params.get('diagram') || Object.keys(diagrams)[0];
  var diagram = diagrams[id];
  if(!diagram){
    document.getElementById('loading').style.display='flex';
    document.getElementById('loading').textContent='UNKNOWN DIAGRAM: '+id;
    return;
  }

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
  document.body.appendChild(renderer.domElement);

  createOrbitControls(camera, renderer, cam);

  // Build diagram
  var result = diagram.build(scene, helpers);
  var explodeCtrl = createExplodeController(result.groups, result.explodeSmallY, result.explodeGrid);
  createHoverSystem(camera, renderer, result.hoverData);
  createTabs(diagram.tabs, result.tabStates, explodeCtrl);

  // Render loop
  (function animate(){
    requestAnimationFrame(animate);
    renderer.render(scene, camera);
  })();

  window.addEventListener('resize',function(){
    camera.aspect=innerWidth/innerHeight;
    camera.updateProjectionMatrix();
    renderer.setSize(innerWidth, innerHeight);
  });
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
