// The homepage's turntable: the phones turn on the page one after another.
// Each phone turns in from the back, slows almost to a stop as the phone
// faces the reader, and speeds up again as the phone turns away; while the
// phone's back is toward the reader, the phone becomes the next phone: the
// two turn together and cross-fade, and their sizes meet halfway (through
// the back of the turn, the leaving phone grows or shrinks toward the size
// between the two, and the next phone grows or shrinks from that size to
// its own).  The cross-fade blends two whole pictures, each phone drawn
// alone with the phone's own shadow, so that neither phone shows through
// itself.  The models are glTF files built by
// tools/phone-models, with models.json, which lists the models in the order
// they turn; the box's data-models attribute is their directory.  The
// phones keep their true sizes relative to one another.  With reduced
// motion, one phone stands still, facing the reader.

import {
  DirectionalLight,
  HalfFloatType,
  Mesh,
  NoBlending,
  type Object3D,
  OrthographicCamera,
  PCFShadowMap,
  PerspectiveCamera,
  PlaneGeometry,
  PMREMGenerator,
  Scene,
  ShaderMaterial,
  ShadowMaterial,
  SRGBColorSpace,
  Vector3,
  WebGLRenderer,
  WebGLRenderTarget,
} from "three";
import { RoomEnvironment } from "three/addons/environments/RoomEnvironment.js";
import { GLTFLoader } from "three/addons/loaders/GLTFLoader.js";

/** models.json, as tools/phone-models writes the file. */
interface Manifest {
  models: { model: string; file: string; box: [number, number, number, number, number, number] }[];
  /** The model that stands still, with reduced motion, and turns first. */
  still: string;
}

const TURN = 8; // seconds per phone: one whole turn
const DRIFT = 0.05; // the share of the turn's speed that remains facing front
const ELEVATION = 33; // degrees: the camera looks down at the desk
const AZIMUTH = -10; // degrees: and from a little to the phone's left
const UP = new Vector3(0, 1, 0);
const BLEND = 0.2; // the share of the turn on each side of the change over which the sizes meet
const FADE = 0.06; // the share of the turn on each side of the change over which the phones cross-fade

function smoothstep(a: number, b: number, t: number): number {
  const x = Math.min(1, Math.max(0, (t - a) / (b - a)));
  return x * x * (3 - 2 * x);
}

/** The turn: from the back (-180 degrees) through the front (0) to the
 * back again.  The angle is an odd polynomial of the turn's progress, so
 * the phone turns fast at the back and slowly at the front. */
function angle(progress: number): number {
  const v = 2 * progress - 1;
  return Math.PI * (DRIFT * v + (1 - DRIFT) * v ** 5);
}

const radians = (degrees: number) => (degrees * Math.PI) / 180;

async function start(root: HTMLElement): Promise<void> {
  const here = new URL(root.dataset.models ?? "/phones/", location.href);
  const manifest = (await (await fetch(new URL("models.json", here))).json()) as Manifest;
  const still = matchMedia("(prefers-reduced-motion: reduce)").matches;

  const renderer = new WebGLRenderer({ antialias: true, alpha: true });
  renderer.outputColorSpace = SRGBColorSpace;
  renderer.shadowMap.enabled = true;
  renderer.shadowMap.type = PCFShadowMap;
  renderer.setClearColor(0x000000, 0);
  root.append(renderer.domElement);

  const scene = new Scene();
  scene.environment = new PMREMGenerator(renderer).fromScene(new RoomEnvironment(), 0.04).texture;
  scene.environmentIntensity = 0.18;

  // The studio's lights: a key light above and to the left, which casts
  // the shadow, a cool fill from the right, a top light, and a warm rim
  // light from behind.
  const light = (x: number, y: number, z: number, intensity: number, color = 0xffffff) => {
    const l = new DirectionalLight(color, intensity);
    l.position.set(x, y, z);
    scene.add(l);
    return l;
  };
  const key = light(-4.2, 6.6, 3.6, 2.6);
  light(4.5, 2.0, 4.0, 0.9, 0xebf5ff);
  light(0.5, 6.0, -2.5, 2.2);
  light(2.5, 5.0, -4.0, 2.0, 0xfff7eb);
  key.castShadow = true;
  key.shadow.mapSize.set(2048, 2048);
  key.shadow.radius = 5;
  key.shadow.bias = -0.0004;
  key.shadow.normalBias = 0.002;
  Object.assign(key.shadow.camera, { left: -0.4, right: 0.4, top: 0.4, bottom: -0.4, near: 0.5, far: 12 });

  // The desk: only the shadow shows.
  const desk = new Mesh(new PlaneGeometry(3, 3), new ShadowMaterial({ opacity: 0.32 }));
  desk.rotation.x = -Math.PI / 2;
  desk.receiveShadow = true;
  scene.add(desk);

  // The camera: one distance and lens for every phone, so that the largest
  // phone stays in the picture all the way around.
  const camera = new PerspectiveCamera(24, 1, 0.05, 20);
  const el = radians(ELEVATION);
  const az = radians(AZIMUTH);
  const toward = new Vector3(Math.sin(az) * Math.cos(el), Math.sin(el), Math.cos(az) * Math.cos(el));
  const across = UP.clone().cross(toward).normalize();
  const upward = toward.clone().cross(across);
  const height = Math.max(...manifest.models.map((m) => m.box[4]));
  const target = new Vector3(0, height * 0.42, 0);
  const corners: Vector3[] = [];
  for (const { box } of manifest.models) {
    const [x0, y0, z0, x1, y1, z1] = box;
    for (const x of [x0, x1]) for (const y of [y0, y1]) for (const z of [z0, z1]) corners.push(new Vector3(x, y, z));
  }
  const frame = () => {
    const { clientWidth: w, clientHeight: h } = root;
    renderer.setPixelRatio(Math.min(devicePixelRatio, 2));
    renderer.setSize(w, h, false);
    camera.aspect = w / h;
    // The distance at which every corner of every phone, at every angle of
    // the turn, is in the picture, with a margin.
    const tanV = Math.tan(radians(camera.fov / 2)) * 0.94;
    const tanH = tanV * camera.aspect;
    let distance = 0;
    const p = new Vector3();
    for (let a = 0; a < 360; a += 10) {
      for (const c of corners) {
        p.copy(c).applyAxisAngle(UP, radians(a)).sub(target);
        const need = Math.max(Math.abs(p.dot(across)) / tanH, Math.abs(p.dot(upward)) / tanV) + p.dot(toward);
        distance = Math.max(distance, need);
      }
    }
    camera.position.copy(target).addScaledVector(toward, distance);
    camera.lookAt(target);
    camera.updateProjectionMatrix();
  };
  const render = () => renderer.render(scene, camera);

  // The cross-fade: each phone drawn alone into a picture of the picture's
  // own, then the two pictures mixed onto the page.
  const pictures = [0, 1].map(() => new WebGLRenderTarget(1, 1, { type: HalfFloatType, samples: 4 }));
  const mix = new ShaderMaterial({
    uniforms: { a: { value: pictures[0].texture }, b: { value: pictures[1].texture }, k: { value: 0 } },
    vertexShader: "varying vec2 vUv;\nvoid main() { vUv = uv; gl_Position = vec4(position.xy, 0.0, 1.0); }",
    fragmentShader:
      "uniform sampler2D a;\nuniform sampler2D b;\nuniform float k;\nvarying vec2 vUv;\n" +
      "void main() { gl_FragColor = linearToOutputTexel(mix(texture2D(a, vUv), texture2D(b, vUv), k)); }",
    depthTest: false,
    depthWrite: false,
    blending: NoBlending,
  });
  const mixScene = new Scene();
  const mixQuad = new Mesh(new PlaneGeometry(2, 2), mix);
  mixQuad.frustumCulled = false;
  mixScene.add(mixQuad);
  const mixCamera = new OrthographicCamera(-1, 1, 1, -1, 0, 1);
  const crossFade = (a: Object3D, b: Object3D, k: number) => {
    for (const [phone, picture] of [
      [a, pictures[0]],
      [b, pictures[1]],
    ] as const) {
      phone.visible = true;
      renderer.setRenderTarget(picture);
      renderer.render(scene, camera);
      phone.visible = false;
    }
    renderer.setRenderTarget(null);
    mix.uniforms.k.value = k;
    renderer.render(mixScene, mixCamera);
  };
  const sizePictures = () => {
    const { clientWidth: w, clientHeight: h } = root;
    const r = renderer.getPixelRatio();
    for (const p of pictures) p.setSize(Math.max(1, Math.round(w * r)), Math.max(1, Math.round(h * r)));
  };
  frame();
  sizePictures();
  new ResizeObserver(() => {
    frame();
    sizePictures();
    render();
  }).observe(root);

  const loader = new GLTFLoader();
  const loading = new Map<number, Promise<Object3D>>();
  const load = (i: number): Promise<Object3D> => {
    let p = loading.get(i);
    if (!p) {
      p = loader.loadAsync(new URL(manifest.models[i].file, here).href).then((gltf) => {
        gltf.scene.traverse((o) => {
          if (o instanceof Mesh) {
            const material = Array.isArray(o.material) ? o.material[0] : o.material;
            o.castShadow = material.userData.castShadow !== false;
            o.receiveShadow = true;
          }
        });
        return gltf.scene;
      });
      loading.set(i, p);
    }
    return p;
  };

  const count = manifest.models.length;
  let index = Math.max(0, manifest.models.findIndex((m) => m.model === manifest.still));
  const shown = await load(index);
  scene.add(shown);
  if (still) {
    render();
    return;
  }

  // The others load while the first turns; every phone stays in the scene,
  // hidden but for the phones being drawn.
  const ready = new Map<number, Object3D>([[index, shown]]);
  for (let i = 0; i < count; i++) {
    void load(i).then((p) => {
      if (!ready.has(i)) {
        p.visible = false;
        scene.add(p);
        ready.set(i, p);
      }
    });
  }
  // Each model's size along x, y, and z.
  const size = manifest.models.map(({ box }) => new Vector3(box[3] - box[0], box[4] - box[1], box[5] - box[2]));
  /** The scale that gives model `i` the size halfway between models `i`
   * and `j`, at the share `k` of the way there. */
  const blend = (i: number, j: number, k: number) => {
    const half = size[i].clone().add(size[j]).multiplyScalar(0.5).divide(size[i]);
    return new Vector3(1, 1, 1).lerp(half, k);
  };
  const pose = (phone: Object3D, progress: number, scale: Vector3) => {
    phone.rotation.y = angle(progress);
    phone.scale.copy(scale);
  };
  // The model shown before the current one, for the change into the
  // current one.
  let previous = index;

  // The first phone starts a little before facing the reader.
  const origin = performance.now() / 1000 - TURN * 0.42;
  let turns = 0;
  let visible = true;
  const tick = (now: number) => {
    if (!visible) return;
    const t = now / 1000 - origin;
    const n = Math.floor(t / TURN);
    if (n > turns) {
      turns = n;
      // The phone's back is toward the reader: the next phone, once the
      // next has loaded, takes the turntable.
      previous = index;
      if (ready.has((index + 1) % count)) index = (index + 1) % count;
    }
    const progress = t / TURN - n;
    const current = ready.get(index)!;
    for (const p of ready.values()) p.visible = false;
    // Near the change, two phones: the leaving one (a) and the coming one
    // (b), turning together; `k` is how far the cross-fade has gone.
    let a = current;
    let b: Object3D | undefined;
    let k = 0;
    if (progress >= 0.5) {
      const nextIndex = (index + 1) % count;
      const next = ready.get(nextIndex);
      const s = smoothstep(1 - BLEND, 1, progress);
      pose(current, progress, blend(index, next ? nextIndex : index, s));
      if (next && progress > 1 - FADE) {
        pose(next, progress - 1, blend(nextIndex, index, s));
        b = next;
        k = smoothstep(1 - FADE, 1 + FADE, progress);
      }
    } else {
      const before = ready.get(previous);
      const s = 1 - smoothstep(0, BLEND, progress);
      pose(current, progress, blend(index, previous, s));
      if (before && previous !== index && progress < FADE) {
        pose(before, progress + 1, blend(previous, index, s));
        a = before;
        b = current;
        k = smoothstep(1 - FADE, 1 + FADE, progress + 1);
      }
    }
    if (b && k > 0 && k < 1) {
      crossFade(a, b, k);
    } else {
      (b && k >= 1 ? b : a).visible = true;
      render();
    }
    requestAnimationFrame(tick);
  };
  new IntersectionObserver(([e]) => {
    const was = visible;
    visible = e.isIntersecting;
    if (visible && !was) requestAnimationFrame(tick);
  }).observe(root);
  requestAnimationFrame(tick);
}

for (const root of document.querySelectorAll<HTMLElement>(".phone-turntable")) {
  start(root).catch((e: unknown) => console.error("phone turntable:", e));
}
