import * as THREE from "three";
import { GLTFLoader } from "./vendor/vrm/loaders/GLTFLoader.js";
import { VRMLoaderPlugin, VRMMetaLoaderPlugin } from "@pixiv/three-vrm";

const clamp01 = value => Math.max(0, Math.min(1, Number(value) || 0));

export async function createVrmRuntime(canvas, url, config = {}) {
  const width = Number(config.width) || 285;
  const height = Number(config.height) || 315;
  const renderer = new THREE.WebGLRenderer({
    canvas,
    alpha: true,
    antialias: true,
    premultipliedAlpha: true,
  });
  renderer.setPixelRatio(Math.min(window.devicePixelRatio || 1, 2));
  renderer.setClearColor(0x000000, 0);
  renderer.outputColorSpace = THREE.SRGBColorSpace;
  renderer.setSize(width, height, false);

  const scene = new THREE.Scene();
  scene.add(new THREE.HemisphereLight(0xffffff, 0x334155, 2.2));
  // PicoAgentのキャラクター表示は顔と手元が読める上半身を標準にする。
  // VRMの全身をcanvasへ収めると常駐サイズでは表情が潰れるため、腰上を中心に固定する。
  const upperBody = config.framing !== "full-body";
  const halfWidth = upperBody ? 0.62 : 0.72;
  const halfHeight = upperBody ? 0.64 : 0.92;
  const targetY = upperBody ? 1.3 : 0.9;
  const camera = new THREE.OrthographicCamera(-halfWidth, halfWidth, halfHeight, -halfHeight, 0.1, 10);
  camera.position.set(0, targetY, 3);
  camera.lookAt(0, targetY, 0);

  const loader = new GLTFLoader();
  loader.register(parser => new VRMLoaderPlugin(parser, {
    metaPlugin: new VRMMetaLoaderPlugin(parser, { acceptLicenseUrls: [config.licenseUrl].filter(Boolean) }),
  }));
  const gltf = await loader.loadAsync(url);
  const vrm = gltf.userData.vrm;
  if (!vrm) throw new Error("VRM 1.0モデルとして読み込めませんでした");
  scene.add(vrm.scene);

  const boneRest = new Map();
  for (const name of ["chest", "head", "leftUpperArm", "rightUpperArm"]) {
    const bone = vrm.humanoid?.getNormalizedBoneNode(name);
    if (bone) boneRest.set(name, { bone, quaternion: bone.quaternion.clone() });
  }

  let stopped = false;
  let mouth = 0;
  let state = "idle";
  let elapsed = 0;
  let lastNow = performance.now();
  let nextBlinkAt = lastNow + 1800;
  let blinkStart = Number.NEGATIVE_INFINITY;
  const setExpression = (name, value) => vrm.expressionManager?.setValue(name, clamp01(value));
  const frame = now => {
    if (stopped) return;
    const delta = Math.min((now - lastNow) / 1000, 1 / 15);
    lastNow = now;
    elapsed += delta;
    if (now >= nextBlinkAt) {
      blinkStart = now;
      nextBlinkAt = now + 2600 + Math.random() * 1300;
    }
    const blinkProgress = (now - blinkStart) / 340;
    const blink = blinkProgress > 0 && blinkProgress < 1
      ? Math.sin(Math.PI * blinkProgress) ** 1.45
      : 0;
    setExpression("blink", blink);
    for (const name of ["happy", "sad", "relaxed"]) setExpression(name, 0);
    if (state === "sad") setExpression("sad", 0.7);
    else if (state === "pleased" || state === "shy") setExpression("happy", 0.55);
    else if (state === "idle") setExpression("relaxed", 0.08);
    for (const name of ["aa", "ih", "ou", "ee", "oh"]) setExpression(name, 0);
    setExpression("aa", mouth);
    setExpression("breath", 0.5 - 0.5 * Math.cos(elapsed * 1.25));

    const sway = Math.sin(elapsed * 0.8);
    const rotations = {
      chest: 0.006 * sway,
      head: 0.009 * Math.sin(elapsed * 0.8 - 0.4),
      leftUpperArm: Number(config.leftArmAngle ?? -1.22),
      rightUpperArm: Number(config.rightArmAngle ?? 1.22),
    };
    for (const [name, { bone, quaternion }] of boneRest) {
      bone.quaternion.copy(quaternion);
      bone.rotateZ(rotations[name] || 0);
    }
    vrm.update(delta);
    renderer.render(scene, camera);
    requestAnimationFrame(frame);
  };
  requestAnimationFrame(frame);

  return {
    setMouth(value) { mouth = clamp01(value); },
    setState(value) { state = String(value || "idle"); },
    focus(x, y) {
      setExpression("lookLeft", Math.max(0, x));
      setExpression("lookRight", Math.max(0, -x));
      setExpression("lookUp", Math.max(0, y));
      setExpression("lookDown", Math.max(0, -y));
    },
    destroy() {
      stopped = true;
      scene.remove(vrm.scene);
      vrm.scene.traverse(object => {
        object.geometry?.dispose?.();
        if (Array.isArray(object.material)) object.material.forEach(material => material.dispose?.());
        else object.material?.dispose?.();
      });
      renderer.dispose();
    },
  };
}
