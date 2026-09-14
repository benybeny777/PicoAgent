// PMXを変換せず読み込み、姿勢・モーフだけを表示側で制御する。
(() => {
  'use strict';
  const instances = new WeakMap();
  const vendorUrl = new URL('vendor/mmd/mmd-runtime.js', document.currentScript?.src || new URL('mmd-renderer.js', document.baseURI)).href;
  let vendorPromise;
  const clamp = value => Math.max(0, Math.min(1, Number(value) || 0));
  const stateAliases = Object.freeze({
    success: 'happy', pleased: 'happy', shy: 'happy', concern: 'sad', failed: 'sad', error: 'sad',
    searching: 'thinking', working: 'thinking', listening: 'thinking',
  });
  function normalizeState(value) {
    const state = String(value || 'idle').trim().toLowerCase();
    return state && state.length <= 32 ? state : 'idle';
  }
  function normalizePoseMap(value) {
    if (!value || typeof value !== 'object' || Array.isArray(value)) return {};
    return Object.fromEntries(Object.entries(value).slice(0, 128).flatMap(([name, angles]) => {
      const bone = String(name).trim();
      if (!bone || bone.length > 128 || !Array.isArray(angles) || angles.length !== 3 || !angles.every(Number.isFinite)) return [];
      return [[bone, angles.map(angle => Math.max(-180, Math.min(180, angle)))]];
    }));
  }
  function normalizeExpressionProfile(value) {
    if (!value || typeof value !== 'object' || Array.isArray(value)) return null;
    const source = value.morphs && typeof value.morphs === 'object' && !Array.isArray(value.morphs) ? value.morphs : value;
    const morphs = Object.fromEntries(Object.entries(source).slice(0, 64).flatMap(([name, weight]) => {
      const morph = String(name).trim();
      return morph && morph.length <= 128 && Number.isFinite(weight) ? [[morph, clamp(weight)]] : [];
    }));
    return { morphs, autoBlink: value.autoBlink !== false };
  }
  function normalizeProfiles(value, normalizer) {
    if (!value || typeof value !== 'object' || Array.isArray(value)) return {};
    return Object.fromEntries(Object.entries(value).slice(0, 16).flatMap(([name, profile]) => {
      const state = normalizeState(name), normalized = normalizer(profile);
      return normalized === null ? [] : [[state, normalized]];
    }));
  }
  function profileFor(profiles, state) {
    const exact = normalizeState(state);
    if (Object.hasOwn(profiles, exact)) return profiles[exact];
    const alias = stateAliases[exact];
    return alias && Object.hasOwn(profiles, alias) ? profiles[alias] : null;
  }
  function combinePose(base, overlay) {
    const combined = new Map();
    for (const [name, angles] of Object.entries(base || {})) combined.set(name, [...angles]);
    for (const [name, angles] of Object.entries(overlay || {})) {
      const current = combined.get(name) || [0, 0, 0];
      combined.set(name, angles.map((angle, index) => Math.max(-180, Math.min(180, current[index] + angle))));
    }
    return combined;
  }
  function vendor() {
    if (window.PicoMMDVendor) return Promise.resolve(window.PicoMMDVendor);
    if (!vendorPromise) vendorPromise = new Promise((resolve, reject) => {
      const script = document.createElement('script');
      script.src = vendorUrl;
      script.onload = () => window.PicoMMDVendor ? resolve(window.PicoMMDVendor) : reject(new Error('MMDランタイムの読み込みに失敗しました'));
      script.onerror = () => { script.remove(); reject(new Error('MMDランタイムがありません。cargo xtask setup mmd を実行してください')); };
      document.head.append(script);
    }).catch(error => { vendorPromise = null; throw error; });
    return vendorPromise;
  }
  function texturePath(value) {
    // PMX内のWindows区切りだけを正規化。モデル外参照とURL指定は取得前に拒否する。
    const source = String(value).replaceAll('\\', '/');
    if (!source || source.startsWith('/') || /[:\u0000-\u001f]/u.test(source)) throw new Error(`モデル外のテクスチャ参照です: ${value}`);
    const segments = source.split('/');
    if (segments.some(part => part === '..' || /%(?:2e|2f|5c|00)/iu.test(part))) throw new Error(`モデル外のテクスチャ参照です: ${value}`);
    return segments.filter(part => part && part !== '.').map(encodeURIComponent).join('/');
  }
  async function create(canvas, { modelUrl, config = {}, interactive = false, signal } = {}) {
    instances.get(canvas)?.dispose();
    let disposed = false, engine, scene, observer, container;
    let motion = false, mouth = 0, state = 'idle', pose = config.defaultPose === 'original' ? 'original' : 'natural';
    const dispose = () => {
      if (disposed) return;
      disposed = true;
      observer?.disconnect();
      signal?.removeEventListener('abort', dispose);
      engine?.stopRenderLoop();
      scene?.dispose();
      container?.dispose();
      engine?.dispose();
      if (instances.get(canvas)?.dispose === dispose) instances.delete(canvas);
    };
    instances.set(canvas, { dispose });
    signal?.addEventListener('abort', dispose, { once: true });
    const ensureActive = () => {
      if (disposed || signal?.aborted) throw new DOMException('MMDの読み込みを中断しました', 'AbortError');
    };
    try {
      ensureActive();
      const B = await vendor();
      ensureActive();
      const url = new URL(modelUrl, document.baseURI);
      const localAsset = !url.port && !url.username && !url.password && (
        (url.protocol === 'asset:' && url.hostname === 'localhost') ||
        (url.protocol === 'http:' && url.hostname === 'asset.localhost')
      );
      const sameOrigin = url.origin !== 'null' && url.origin === new URL(document.baseURI).origin;
      if (!/\.pmx$/iu.test(url.pathname) || url.search || url.hash || url.username || url.password || !(sameOrigin || localAsset)) throw new Error('同じアプリ内のPMXモデルを指定してください');
      engine = new B.Engine(canvas, true, { alpha: true, premultipliedAlpha: false, preserveDrawingBuffer: true, stencil: true });
      // CSS寸法の引き伸ばしではなく、実画面の画素密度でモデルを描き直す。
      let appliedPixelRatio = 0;
      const resize = () => {
        appliedPixelRatio = Math.max(1, window.devicePixelRatio || 1);
        engine.setHardwareScalingLevel(1 / appliedPixelRatio);
        engine.resize();
      };
      B.SdefInjector.OverrideEngineCreateEffect(engine);
      scene = new B.Scene(engine);
      scene.clearColor = new B.Color4(0, 0, 0, 0);
      scene.ambientColor = new B.Color3(0.65, 0.65, 0.65);
      const fill = new B.HemisphericLight('mmd-fill', new B.Vector3(0, 1, 0), scene);
      fill.intensity = 0.7;
      fill.groundColor = new B.Color3(1, 1, 1);
      new B.DirectionalLight('mmd-key', new B.Vector3(-0.3, -0.35, 1), scene).intensity = 0.6;
      const camera = new B.ArcRotateCamera('mmd-camera', -Math.PI / 2, Math.PI / 2, 25, new B.Vector3(0, 14, 0), scene);
      camera.minZ = 0.05;
      camera.wheelPrecision = 35;
      camera.lowerRadiusLimit = 1;
      camera.upperRadiusLimit = 100;
      if (interactive) camera.attachControl(canvas, true);
      class LocalMaterialBuilder extends B.MmdStandardMaterialBuilder {
        errors = [];
        buildMaterials(...args) {
          args[3] = args[3].map(texturePath);
          const logger = args[12];
          args[12] = {
            log: message => logger.log(message), warn: message => logger.warn(message),
            error: message => { this.errors.push(String(message)); logger.error(message); },
          };
          return super.buildMaterials(...args);
        }
      }
      const materialBuilder = new LocalMaterialBuilder();
      container = await B.LoadAssetContainerAsync(url.href, scene, {
        pluginExtension: '.pmx', pluginOptions: { mmdmodel: { materialBuilder, useSdef: true } },
      });
      if (disposed) { container.dispose(); ensureActive(); }
      if (materialBuilder.errors.length) throw new Error(`MMDテクスチャを読み込めません: ${materialBuilder.errors.slice(0, 3).join('; ')}`);
      container.addAllToScene();
      const mesh = container.meshes.find(item => item.metadata?.isMmdModel) || container.meshes[0];
      if (!mesh?.metadata?.skeleton) throw new Error('PMXのボーン情報を読み込めませんでした');
      const morphNames = (mesh.metadata.morphs || []).map(item => item.name);
      // MMD化後は絶対行列APIが置き換わるため、読み込み直後の骨から顔位置を保存する。
      const sourceSkeleton = mesh.metadata.skeleton;
      sourceSkeleton.computeAbsoluteMatrices(true);
      const eyes = ['左目', '右目'].map(name => sourceSkeleton.bones.find(bone => bone.name === name)?.getAbsolutePosition()).filter(Boolean);
      const runtime = new B.MmdRuntime(scene);
      const model = runtime.createMmdModel(mesh, { buildPhysics: false, materialProxyConstructor: B.MmdStandardMaterialProxy });
      const bones = new Map(model.runtimeBones.map(bone => [bone.name, bone.linkedBone]));
      const rest = new Map([...bones].map(([name, bone]) => [name, bone.rotationQuaternion.clone()]));
      const morphSet = new Set(morphNames);
      const choose = (key, candidates) => [config.morphs?.[key], ...candidates].find(name => name && morphSet.has(name));
      const blink = choose('blink', ['まばたき', '瞬き']);
      const mouthMorph = choose('mouth', ['あ', 'あ２', 'A']);
      const smile = choose('happy', ['笑い', 'にこり', '笑顔']);
      const smileCorners = config.morphs?.happy ? [] : (morphSet.has('口角上げ')
        ? ['口角上げ'] : ['左口角上げ', '右口角上げ'].filter(name => morphSet.has(name)));
      const sad = choose('sad', ['困る', '悲しい']);
      const expressionProfiles = normalizeProfiles(config.expressions, normalizeExpressionProfile);
      const configuredMorphs = Object.values(expressionProfiles).flatMap(profile => Object.keys(profile.morphs));
      const affectedMorphs = [...new Set([blink, mouthMorph, smile, sad, ...smileCorners, ...configuredMorphs].filter(name => morphSet.has(name)))];
      const setMorph = (name, value) => { if (name) model.morph.setMorphWeight(name, clamp(value)); };
      let framing = 'bust';
      const positions = mesh.getVerticesData('position');
      let low = Infinity, high = -Infinity;
      for (let i = 1; i < (positions?.length || 0); i += 3) { low = Math.min(low, positions[i]); high = Math.max(high, positions[i]); }
      if (!Number.isFinite(low) || high <= low) { low = 0; high = 20; }
      const height = high - low;
      const faceCenter = eyes.length
        ? eyes.reduce((sum, point) => sum.add(point), B.Vector3.Zero()).scale(1 / eyes.length)
        : new B.Vector3(0, high - height * 0.15, 0);
      faceCenter.y -= height * 0.015;
      function setFraming(kind = 'bust') {
        if (disposed) return;
        framing = kind;
        const head = ['head', 'chat', 'bar'].includes(kind);
        const full = kind === 'full';
        const visibleHeight = height * (full ? 1.08 : head ? 0.24 : 0.48);
        camera.setTarget(head ? faceCenter : new B.Vector3(0, high - visibleHeight * (full ? 0.47 : 0.44), 0));
        const aspect = Math.max(0.2, canvas.clientWidth / Math.max(1, canvas.clientHeight));
        camera.radius = visibleHeight / (2 * Math.tan(camera.fov / 2)) * Math.max(1, 0.65 / aspect);
        camera.alpha = -Math.PI / 2;
        camera.beta = Math.PI / 2;
      }
      // MMDの左右はモデルから見た側。元の回転を基準に合成し、PMXを保存し直さない。
      const natural = normalizePoseMap(config.naturalPose || {
        '左肩': [0, 0, -2], '右肩': [0, 0, 2],
        '左腕': [0, 0, -35], '右腕': [0, 0, 35],
        '左ひじ': [0, 10, 0], '右ひじ': [0, -10, 0],
      });
      const poseProfiles = normalizeProfiles(config.poses, normalizePoseMap);
      const controlledBones = new Set([
        ...Object.keys(natural), ...Object.values(poseProfiles).flatMap(profile => Object.keys(profile)), '上半身', '首',
      ]);
      function rotate(name, angles) {
        const bone = bones.get(name), base = rest.get(name);
        if (!bone || !base) return;
        const [x, y, z] = angles.map(value => Math.max(-180, Math.min(180, value)) * Math.PI / 180);
        bone.rotationQuaternion.copyFrom(base.multiply(B.Quaternion.FromEulerAngles(x, y, z)));
      }
      const start = performance.now();
      let lastUpdateAt = start;
      let currentPose = pose === 'natural' ? combinePose(natural, profileFor(poseProfiles, state)) : new Map();
      const currentExpressions = new Map(affectedMorphs.map(name => [name, 0]));
      function update() {
        if (disposed) return;
        const now = performance.now(), delta = Math.min(0.1, Math.max(0, (now - lastUpdateAt) / 1000));
        lastUpdateAt = now;
        const blend = 1 - Math.exp(-delta * 10);
        for (const [name, bone] of bones) bone.rotationQuaternion.copyFrom(rest.get(name));
        const targetPose = pose === 'natural' ? combinePose(natural, profileFor(poseProfiles, state)) : new Map();
        const nextPose = new Map();
        for (const name of controlledBones) {
          const current = currentPose.get(name) || [0, 0, 0], target = targetPose.get(name) || [0, 0, 0];
          const angles = current.map((value, index) => value + (target[index] - value) * blend);
          if (targetPose.has(name) || angles.some(value => Math.abs(value) > 0.01)) nextPose.set(name, angles);
        }
        currentPose = nextPose;
        const time = (now - start) / 1000;
        const renderedPose = new Map(currentPose);
        if (motion) {
          for (const [name, movement] of [
            ['上半身', [Math.sin(time * 1.5) * 0.7, 0, Math.sin(time * 0.7) * 0.6]],
            ['首', [Math.sin(time * 0.65), Math.sin(time * 0.42) * 2, 0]],
          ]) {
            const base = renderedPose.get(name) || [0, 0, 0];
            renderedPose.set(name, base.map((value, index) => value + movement[index]));
          }
        }
        for (const [name, angles] of renderedPose) rotate(name, angles);

        const configuredExpression = profileFor(expressionProfiles, state);
        const alias = stateAliases[state] || state;
        let targetExpressions = configuredExpression?.morphs || {};
        let suppressBlink = configuredExpression?.autoBlink === false;
        if (!configuredExpression && alias === 'happy') {
          targetExpressions = smileCorners.length
            ? Object.fromEntries(smileCorners.map(name => [name, 0.65]))
            : smile ? { [smile]: 1 } : {};
          suppressBlink = !smileCorners.length;
        }
        if (!configuredExpression && alias === 'sad' && sad) targetExpressions = { [sad]: 0.65 };
        for (const name of affectedMorphs) {
          if (name === blink || name === mouthMorph) continue;
          const current = currentExpressions.get(name) || 0, target = targetExpressions[name] || 0;
          const value = current + (target - current) * blend;
          currentExpressions.set(name, value);
          setMorph(name, value);
        }
        const configuredBlink = blink && Object.hasOwn(targetExpressions, blink);
        if (configuredBlink) {
          const current = currentExpressions.get(blink) || 0, target = targetExpressions[blink];
          const value = current + (target - current) * blend;
          currentExpressions.set(blink, value);
          setMorph(blink, value);
        } else if (motion && !suppressBlink) {
          const cycle = time % 4.7;
          setMorph(blink, cycle < 0.16 ? Math.sin(cycle / 0.16 * Math.PI) : 0);
        } else setMorph(blink, 0);
        setMorph(mouthMorph, mouth);
        model.beforePhysics(null);
        model.afterPhysics();
      }
      setFraming(config.framing || 'bust');
      observer = new ResizeObserver(() => { if (!disposed) { resize(); setFraming(framing); } });
      observer.observe(canvas);
      resize();
      let lastFrame = -Infinity;
      engine.runRenderLoop(() => {
        const now = performance.now();
        if (document.hidden || now - lastFrame < 1000 / 30) return;
        lastFrame = now;
        // CSS寸法が変わらないまま画素密度だけ変わる場合（DPIの違う画面へ移す）は
        // ResizeObserver が動かないため、描画側で追う。
        if (Math.max(1, window.devicePixelRatio || 1) !== appliedPixelRatio) {
          resize();
          setFraming(framing);
        }
        update();
        scene.render();
      });
      const result = {
        setFraming,
        setExpression(name) { if (!disposed) state = normalizeState(name); },
        setMouthLevel(value) { if (!disposed) mouth = clamp(value); },
        setMotionEnabled(enabled) { if (!disposed) { motion = Boolean(enabled); if (!motion) mouth = 0; } },
        setPose(value) { if (!disposed) pose = value === 'original' ? 'original' : 'natural'; },
        capturePreview() { ensureActive(); resize(); update(); scene.render(); return canvas.toDataURL('image/png'); },
        capabilities: {
          mouth: Boolean(mouthMorph), blink: Boolean(blink), happy: Boolean(smile || smileCorners.length), sad: Boolean(sad),
          poseStates: Object.keys(poseProfiles), expressionStates: Object.keys(expressionProfiles), physics: false,
        },
        dispose,
      };
      instances.set(canvas, result);
      return result;
    } catch (error) { dispose(); throw error; }
  }
  window.PicoMMD = Object.freeze({ create, normalizeState, normalizePoseMap, normalizeExpressionProfile, normalizeProfiles, profileFor, combinePose });
})();
