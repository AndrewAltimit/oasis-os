// PSP Universal USB-C Adapter — Passive Bridge
// Uses BlueprintEngine.buildPSP() for the shared PSP body component

(function(){
var h; // helpers shorthand

BlueprintEngine.register('psp-usb-adapter', {
  title: 'PSP Universal USB-C Adapter',
  subtitle: 'Passive Bridge &bull; Power Pads + Mini-B Data &bull; Single USB-C Receptacle',
  backLink: {href: 'journal/05-psp-usb-research.html', text: '&larr; Entry 05: PSP USB Research'},
  dims: 'PSP top edge: ~82mm wide\nMini-B port: 8x4mm\nPower pads: ~3x5mm each\nUSB-C receptacle: 8.3x2.5mm',
  legend: [
    {color:'#fa0', label:'VBUS (5V from power pads)'},
    {color:'#c4f', label:'D+/D- (Mini-B passthru)'},
    {color:'#888', label:'Mechanical (screws, pogo pins)'},
    {color:'#4f4', label:'CC pulldown resistors (5.1k)'},
    {color:'#4af', label:'USB-C receptacle'}
  ],
  camera: {r:22, phi:Math.PI/2.8, theta:Math.PI/12, target:[0,6,0]},
  tabs: [
    {id:'assembled', label:'ASSEMBLED'},
    {id:'exploded', label:'EXPLODED VIEW'}
  ],

  build: function(scene, helpers){
    h = helpers;
    var G = {};
    ['psp','adapter','usbc_device','wires','dims'].forEach(function(k){
      G[k] = new THREE.Group(); scene.add(G[k]);
    });

    // Shared PSP body component
    var psp = BlueprintEngine.buildPSP(G.psp);
    var teY = psp.topY; // PSP top edge Y
    var pW = psp.pW, pH = psp.pH, pD = psp.pD;

    // PSP labels
    G.psp.add(h.tag('PSP-3001', v(-pW/2-2, pH/2, 0), '#556677', v(-pW/2, pH/2, 0)));
    G.psp.add(h.tag('169.4 x 71.4 x 18.6mm', v(-pW/2-2, pH/2-0.7, 0), '#334455'));

    // Adapter + device + wires
    var adapter = buildAdapter(G.adapter, teY, pD);
    buildDevice(G.usbc_device, adapter.topY);
    buildTraces(G.wires, teY, adapter);
    buildDims(G.dims, pW, pH, pD);

    return {
      groups: G,
      explodeSmallY: {psp:-4, adapter:3, usbc_device:8, wires:0, dims:0},
      explodeGrid: {
        psp:{x:0,y:-8,z:0}, adapter:{x:0,y:0,z:0},
        usbc_device:{x:0,y:10,z:0}, wires:{x:0,y:0,z:0}, dims:{x:0,y:0,z:0}
      },
      hoverData: [
        {group:G.psp, label:'PSP Body (Top Edge)', desc:'PSP-2000/3000 top edge with Mini-B USB port (data only) and proprietary side contact pads. The 5V and GND pads flank the Mini-B port on both sides &mdash; discovered through Go!Cam teardown.<br><span class="dim">Sony never drove 5V on VBUS &mdash; accessories use these pads</span>'},
        {group:G.adapter, label:'Passive Carrier Adapter', desc:'Clip-on module that bridges PSP proprietary connectors to standard USB-C. Pogo pins contact power pads, Mini-B male passes through data. Carrier PCB routes both to a single USB-C receptacle with 5.1k CC pulldowns.<br><span class="dim">~$7-11 per unit, no active components, ENIG finish</span>'},
        {group:G.usbc_device, label:'Any USB-C Device', desc:'The adapter accepts any standard USB-C device &mdash; Luckfox Pico, Raspberry Pi Zero, or any SBC. The device acts as USB host, the PSP as USB device. Power comes from PSP\'s side pads, data from Mini-B passthrough.<br><span class="dim">Custom sceUsbbd driver on PSP: ~100 FPS / 28 MB/s bulk transfer</span>'},
        {group:G.wires, label:'Signal Routing', desc:'VBUS: 5V from left+right power pads routed to USB-C VBUS pin. D+/D-: Mini-B data lines passed through to USB-C data pins. CC1/CC2: 5.1k pulldowns to GND for USB-C device identification.<br><span class="dim">All passive &mdash; no level shifters or active components needed</span>'}
      ],
      tabStates: {
        'assembled': {note: 'Assembled view: adapter clipped onto PSP\nPogo pins contact power pads\nMini-B plug inserted for data', explode: 0},
        'exploded': {note: 'Exploded view: PSP, adapter, and\nexternal device separated for clarity', explode: 80}
      }
    };
  }
});

// Returns { topY, botY, centerY, width, height }
// Layout matches PSP top-edge hardware (from Go!Cam reference photos):
//   [screw x=-2.0] [5V x=-1.1] [GND x=-0.7] [Mini-B x=0] [GND x=0.7] [5V x=1.1] [screw x=2.0]
function buildAdapter(G, teY, pD){
  var aW = 5, aH = 3, aD = pD+1;
  var aBot = teY + 0.15;
  var aCY = aBot + aH/2;
  var aTop = aBot + aH;

  // Shell (spans screw-to-screw width plus margin)
  var shell = h.wireBox(aW, aH, aD, h.lm(0x667788), h.fm(0x112233,0.15));
  shell.position.set(0, aCY, 0); G.add(shell);

  // Carrier PCB (inside shell)
  var pcb = h.wireBox(aW-0.6, 0.3, pD-0.3, h.lm(0x22aa66), h.fm(0x0a2818,0.4));
  pcb.position.set(0, aCY+0.3, 0); G.add(pcb);

  // USB-C receptacle (top of adapter)
  var usbc = h.wireBox(1.0, 0.4, 0.7, h.lm(0x4488ff), h.fm(0x112840,0.5));
  usbc.position.set(0, aTop, 0); G.add(usbc);

  // CC pulldown resistors (on PCB, flanking USB-C)
  [-0.7, 0.7].forEach(function(x){
    var r = h.wireBox(0.15, 0.12, 0.12, h.lm(0x44ff44), h.fm(0x22aa22,0.5));
    r.position.set(x, aCY+0.45, 0); G.add(r);
  });

  // Pogo pins — one per side, align with PSP 5V power pads at ±1.0
  [-1.0, 1.0].forEach(function(x){
    var p = h.wireBox(0.12, 0.7, 0.12, h.lm(0xccaa00));
    p.position.set(x, aBot-0.2, 0); G.add(p);
  });

  // Mini-B male plug (center, aligns with PSP Mini-B port)
  var plug = h.wireBox(0.7, 0.6, 0.5, h.lm(0xcc44ff));
  plug.position.set(0, aBot-0.15, 0); G.add(plug);

  // Screw mounts — align with PSP screw holes (x=±2.0)
  [-2.0, 2.0].forEach(function(x){
    var s = h.cyl(0.15, aH+0.8, 6, h.lm(0x888888));
    s.position.set(x, aCY-0.2, 0); G.add(s);
  });

  // Labels
  G.add(h.tag('ADAPTER SHELL', v(aW/2+1.5, aCY+1, 0), '#667788', v(aW/2, aCY, 0)));
  G.add(h.tag('Passive carrier PCB', v(aW/2+1.5, aCY+0.3, 0), '#22aa66', v(aW/2-0.3, aCY+0.3, 0)));
  G.add(h.tag('USB-C receptacle', v(aW/2+1.5, aTop+0.3, 0), '#4488ff', v(0.7, aTop, 0)));
  G.add(h.tag('5.1k CC pulldowns', v(aW/2+1.5, aCY+0.5, 0), '#44ff44'));
  G.add(h.tag('Pogo pins (5V)', v(-aW/2-1.5, aBot-0.2, 0), '#ccaa00', v(-1.0, aBot-0.2, 0)));
  G.add(h.tag('Mini-B male', v(-aW/2-1.5, aBot-0.8, 0), '#cc44ff', v(-0.5, aBot-0.15, 0)));
  G.add(h.tag('Screw mount', v(-aW/2-1.5, aCY, 0), '#888888', v(-2.0, aCY, 0)));

  return {topY: aTop, botY: aBot, centerY: aCY, width: aW, height: aH};
}

function buildDevice(G, adapterTopY){
  var dY = adapterTopY + 0.3;
  var box = h.wireBox(4, 2.5, 3, h.lm(0x44aaff), h.fm(0x112840,0.25));
  box.position.set(0, dY+2.5, 0); G.add(box);
  var plug = h.wireBox(0.5, 1, 0.3, h.lm(0x888888));
  plug.position.set(0, dY+0.7, 0); G.add(plug);
  G.add(h.tag('ANY USB-C DEVICE', v(0, dY+4.2, 0), '#44aaff', v(0, dY+3.8, 0)));
  G.add(h.tag('Luckfox, Pi Zero, etc.', v(0, dY+3.5, 0), '#2266aa'));
  G.add(h.tag('USB-C plug', v(-3, dY+0.7, 0), '#888888', v(-0.4, dY+0.7, 0)));
}

function buildTraces(G, teY, adapter){
  // VBUS: power pads (±1.0) → pogo pins → PCB → USB-C
  [-1.0, 1.0].forEach(function(x){
    G.add(h.cable([
      v(x, teY+0.15, 0),
      v(x, adapter.botY, 0),
      v(x*0.5, adapter.centerY+0.3, 0),
      v(x*0.1, adapter.topY, 0)
    ], h.lm(0xffaa00)));
  });
  // D+/D-: Mini-B (center) → USB-C data
  [-0.15, 0.15].forEach(function(x){
    G.add(h.cable([
      v(x, teY+0.3, 0),
      v(x, adapter.centerY, 0),
      v(x, adapter.topY, 0)
    ], h.lm(0xcc44ff)));
  });
}

function buildDims(G, pW, pH, pD){
  G.add(h.dimLine(v(-pW/2, -0.5, pD/2+1.5), v(pW/2, -0.5, pD/2+1.5), '169.4mm'));
  G.add(h.dimLine(v(pW/2+1.5, 0, pD/2+1.5), v(pW/2+1.5, pH, pD/2+1.5), '71.4mm'));
}

function v(x,y,z){ return h.v(x,y,z); }

})();
