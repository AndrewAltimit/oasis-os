// PSP Hard Reset Automation — H-Bridge USB Relay Wiring
// Registers with BlueprintEngine

(function(){
var h; // helpers shorthand, set in build()

BlueprintEngine.register('psp-relay', {
  title: 'PSP Hard Reset Automation',
  subtitle: 'LCUS-2 USB Relay &bull; H-Bridge Wiring &bull; 12V Linear Actuator &bull; CH340 Serial',
  backLink: {href: 'journal/08-remote-dev-automation.html', text: '&larr; Entry 08: Zero-Touch PSP Development'},
  dims: 'USB relay: 50x26mm PCB\nActuator: 10mm stroke, 188N\n12V 2A power supply\nCH340 serial control',
  legend: [
    {color:'#f44', label:'12V power (from adapter)'},
    {color:'#4af', label:'USB data (to desktop PC)'},
    {color:'#4f4', label:'Relay coil control signal'},
    {color:'#fa0', label:'Motor wiring (H-bridge)'},
    {color:'#c4f', label:'Screw terminal connections'}
  ],
  camera: {r:38, phi:Math.PI/3.2, theta:Math.PI/4.5, target:[0,5,0]},
  stateNotes: [
    'Both OFF: extend (push slider)',
    'Both ON: retract (release)',
    'Mixed: motor stopped'
  ],
  tabs: [
    {id:'both-off', label:'BOTH OFF (EXTEND)'},
    {id:'both-on', label:'BOTH ON (RETRACT)'},
    {id:'mixed', label:'MIXED (STOP)'}
  ],

  build: function(scene, helpers){
    h = helpers;
    var G = {};
    ['psu','usb','relay','actuator','wires','dims'].forEach(function(k){
      G[k] = new THREE.Group(); scene.add(G[k]);
    });

    buildPSU(G.psu);
    buildPC(G.usb);
    buildRelay(G.relay);
    buildActuator(G.actuator);
    buildWires(G.wires);
    buildDims(G.dims);

    return {
      groups: G,
      explodeSmallY: {psu:0, usb:0, relay:3, actuator:-2, wires:0, dims:0},
      explodeGrid: {
        psu:{x:-18,y:0,z:-12}, usb:{x:18,y:0,z:-12},
        relay:{x:0,y:0,z:0}, actuator:{x:0,y:0,z:18},
        wires:{x:0,y:0,z:0}, dims:{x:0,y:0,z:0}
      },
      hoverData: [
        {group:G.psu, label:'12V 2A DC Adapter', desc:'Standard 12V 2A power supply with screw terminal end. Powers the actuator through the relay H-bridge. No soldering required &mdash; bare wire into screw terminals.<br><span class="dim">~$8, any polarity-marked adapter works</span>'},
        {group:G.usb, label:'Desktop PC (USB Host)', desc:'Sends serial commands to the CH340 chip on the relay module. Python script toggles relay channels to control actuator direction.<br><span class="dim">python psp_actuator.py reboot / python psp_remote.py hard-reboot</span>'},
        {group:G.relay, label:'2-Channel LCUS USB Relay', desc:'LCUS-2 module with CH340 USB-serial chip. Two SPDT relays wired as H-bridge &mdash; cross-wiring NC/NO terminals reverses motor polarity. Status LEDs show relay state.<br><span class="dim">50x26mm PCB, ~$12, Micro-USB power+data</span>'},
        {group:G.actuator, label:'JQDML Linear Actuator', desc:'12V mini linear actuator, 10mm stroke, 188N force. Pushes the PSP power slider to toggle power. H-bridge control: both OFF=extend, both ON=retract, mixed=stop.<br><span class="dim">~$15, attaches to PSP with 3D-printed bracket</span>'},
        {group:G.wires, label:'H-Bridge Wiring', desc:'Cross-wired relay outputs form an H-bridge. 12V+ connects to relay 1 NC and relay 2 NO. GND connects to relay 1 NO and relay 2 NC. COM terminals connect to actuator motor leads. Polarity reversal drives extend/retract.<br><span class="dim">22 AWG hookup wire + Wago 221 lever connectors</span>'}
      ],
      tabStates: {
        'both-off': {note: 'Both relays OFF: NC contacts closed\nNC1=12V+ via COM1 to motor+\nNC2=GND via COM2 to motor-\nActuator EXTENDS (pushes slider)'},
        'both-on': {note: 'Both relays ON: NO contacts closed\nNO1=GND via COM1 to motor+\nNO2=12V+ via COM2 to motor-\nActuator RETRACTS (releases slider)'},
        'mixed': {note: 'One ON, one OFF:\nBoth COM get same voltage\nMotor STOPPED (braking)'}
      }
    };
  }
});

function buildPSU(G){
  var box = h.wireBox(8,3,5, h.lm(0xffaa22), h.fm(0x2a1a05,0.35));
  box.position.set(-14, 1.5, -6); G.add(box);
  var barrel = h.cyl(0.6, 1.5, 8, h.lm(0x888888));
  barrel.position.set(-14, 1.5, -3.3); barrel.rotation.x = Math.PI/2; G.add(barrel);
  var st1 = h.wireBox(1.2,1,1, h.lm(0x44cc44)); st1.position.set(-12.5, 3.2, -6); G.add(st1);
  var st2 = h.wireBox(1.2,1,1, h.lm(0x44cc44)); st2.position.set(-15.5, 3.2, -6); G.add(st2);
  G.add(h.tag('12V 2A ADAPTER', new THREE.Vector3(-14, 5, -6), '#ffaa22', new THREE.Vector3(-14, 3, -6)));
  G.add(h.tag('Screw terminal end', new THREE.Vector3(-14, 4.2, -6), '#886622'));
}

function buildPC(G){
  var box = h.wireBox(8,5,5, h.lm(0x4488ff), h.fm(0x0a1840,0.25));
  box.position.set(14, 2.5, -6); G.add(box);
  var port = h.wireBox(1.2,0.5,0.8, h.lm(0x4488ff)); port.position.set(11.5, 2, -6); G.add(port);
  G.add(h.tag('USB -> DESKTOP PC', new THREE.Vector3(14, 6, -6), '#4488ff', new THREE.Vector3(14, 5, -6)));
  G.add(h.tag('CH340 serial control', new THREE.Vector3(14, 5.2, -6), '#2255aa'));
}

function buildRelay(G){
  var pcb = h.wireBox(12, 0.8, 7, h.lm(0x22aa66), h.fm(0x0a2818,0.3));
  pcb.position.set(0, 4, 0); G.add(pcb);
  var usb = h.wireBox(1.2, 0.6, 0.8, h.lm(0x888888)); usb.position.set(0, 4.5, -3.2); G.add(usb);

  // Relay cubes
  var r1 = h.wireBox(4, 3.5, 4, h.lm(0x4488cc), h.fm(0x112840,0.4)); r1.position.set(-2.5, 6.5, 0.5); G.add(r1);
  G.add(h.tag('RELAY 1', new THREE.Vector3(-2.5, 9, 0.5), '#4488cc', new THREE.Vector3(-2.5, 8.2, 0.5)));
  var r2 = h.wireBox(4, 3.5, 4, h.lm(0x4488cc), h.fm(0x112840,0.4)); r2.position.set(3.5, 6.5, 0.5); G.add(r2);
  G.add(h.tag('RELAY 2', new THREE.Vector3(3.5, 9, 0.5), '#4488cc', new THREE.Vector3(3.5, 8.2, 0.5)));

  // Terminal blocks — matches PCB silk screen layout
  // Relay 1 (left to right): NO1, NC1, COM1
  ['NO1','NC1','COM1'].forEach(function(l, i){
    var x = -4.5+i*2;
    var t = h.wireBox(1.2,1.5,1.2, h.lm(0x44cc88)); t.position.set(x, 4, 3.8); G.add(t);
    G.add(h.tag(l, new THREE.Vector3(x, 2.8, 4.5), '#44cc88'));
  });
  // Relay 2 (left to right): NO2, NC2, COM2
  ['NO2','NC2','COM2'].forEach(function(l, i){
    var x = 1.5+i*2;
    var t = h.wireBox(1.2,1.5,1.2, h.lm(0x44cc88)); t.position.set(x, 4, 3.8); G.add(t);
    G.add(h.tag(l, new THREE.Vector3(x, 2.8, 4.5), '#44cc88'));
  });

  // LEDs
  var led1 = h.wireBox(0.3,0.3,0.3, h.lm(0xff4444), h.fm(0xff2222,0.5)); led1.position.set(-2.5, 4.6, -2); G.add(led1);
  var led2 = h.wireBox(0.3,0.3,0.3, h.lm(0xff4444), h.fm(0xff2222,0.5)); led2.position.set(3.5, 4.6, -2); G.add(led2);
  G.add(h.tag('2-CH LCUS USB RELAY', new THREE.Vector3(0, 10.5, 0), '#22aa66', new THREE.Vector3(0, 8, 0)));
  G.add(h.tag('CH340 serial control', new THREE.Vector3(0, 9.7, 0), '#117744'));
}

function buildActuator(G){
  var body = h.wireBox(3, 3, 8, h.lm(0xcc8844), h.fm(0x2a1a05,0.35)); body.position.set(0, 1.5, 14); G.add(body);
  var shaft = h.wireBox(0.8, 0.8, 3, h.lm(0xcccccc)); shaft.position.set(0, 1.5, 18.5); G.add(shaft);
  var wr = h.wireBox(0.15,0.15,1.5, h.lm(0xff4444)); wr.position.set(-0.5, 2, 9.5); G.add(wr);
  var wb = h.wireBox(0.15,0.15,1.5, h.lm(0x222222)); wb.position.set(0.5, 2, 9.5); G.add(wb);
  G.add(h.tag('JQDML ACTUATOR', new THREE.Vector3(0, 5, 14), '#cc8844', new THREE.Vector3(0, 3, 14)));
  G.add(h.tag('Pushes PSP power slider', new THREE.Vector3(0, 4.2, 14), '#886622'));
  G.add(h.tag('10mm stroke, 188N', new THREE.Vector3(0, 3.4, 14), '#664411'));
}

function buildWires(G){
  // Terminal positions: NO1=-4.5, NC1=-2.5, COM1=-0.5, NO2=1.5, NC2=3.5, COM2=5.5
  // All terminals at Y=4, Z=3.8

  // === FROM 12V ADAPTER (red wires) ===
  // 12V+ to NC1 (relay 1, normally closed)
  G.add(h.cable([v(-12.5,3.5,-6), v(-12.5,4,-2), v(-2.5,4,3.8)], h.lm(0xff4444)));
  G.add(h.tag('12V+', v(-9, 5, -2), '#ff4444'));
  // 12V+ to NO2 (relay 2, normally open — swapped pattern)
  G.add(h.cable([v(-12.5,3.2,-6), v(-12.5,2.5,-2), v(1.5,2.5,2), v(1.5,4,3.8)], h.lm(0xff4444)));
  G.add(h.tag('12V+', v(-4, 3, 0), '#ff4444'));

  // === FROM 12V ADAPTER (black wires) ===
  // GND to NO1 (relay 1, normally open)
  G.add(h.cable([v(-15.5,3.5,-6), v(-15.5,4,-2), v(-4.5,4,3.8)], h.lm(0x888888)));
  G.add(h.tag('GND', v(-15.5, 5, -3), '#888888'));
  // GND to NC2 (relay 2, normally closed — swapped pattern)
  G.add(h.cable([v(-15.5,3.2,-6), v(-15.5,1.8,-2), v(3.5,1.8,2), v(3.5,4,3.8)], h.lm(0x888888)));
  G.add(h.tag('GND', v(-6, 2.3, 0), '#888888'));

  // === TO ACTUATOR (from COM terminals) ===
  // COM1 → actuator lead (red wire to motor)
  G.add(h.cable([v(-0.5,4,3.8), v(-0.5,4,6), v(-0.5,3,8.5), v(-0.5,2,9.5)], h.lm(0xffaa00)));
  G.add(h.tag('To motor +', v(-2.5, 4.5, 7), '#ffaa00'));
  // COM2 → actuator lead (black wire to motor)
  G.add(h.cable([v(5.5,4,3.8), v(5.5,4,6), v(0.5,3,8.5), v(0.5,2,9.5)], h.lm(0xffaa00)));
  G.add(h.tag('To motor -', v(7, 4.5, 7), '#ffaa00'));

  // === USB cable (PC to relay module) ===
  G.add(h.cable([v(11.5,2,-6), v(8,2,-6), v(2,3,-4), v(0,4.5,-3.2)], h.lm(0x4488ff)));
  G.add(h.tag('USB (Micro-B)', v(6, 3, -5), '#4488ff'));
}

function buildDims(G){
  G.add(h.dimLine(v(-6,0,-4), v(6,0,-4), '~50mm relay PCB'));
}

function v(x,y,z){ return h.v(x,y,z); }

})();
