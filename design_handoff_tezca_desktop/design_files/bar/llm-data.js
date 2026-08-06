// Single source of truth for the local-AI model table and accelerator logic.
// Loaded in the helmet of both TezcaBar and LlmPanel so the bar module and the
// panel can never disagree about the backend, the layer split, or VRAM.
(function () {
  const MODELS = {
    'llama3.1:70b': { name: 'llama3.1:70b', short: 'llama3.1', size: '70b', vram: '18.9G', gpuVram: 21.4,
      ctx:'128k', ctxUsed:'18.2k', meta: 'Q4_K_M · resident · 128k ctx', accel: 'ROCm', offload: '81/81 layers on GPU', layers: '81/81 layers · gfx1100' },
    'nomic-embed-text': { name: 'nomic-embed-text', short: 'nomic', size: '137m', vram: '2.5G', gpuVram: 5.0,
      ctx:'8k', ctxUsed:'0.4k', meta: 'embeddings · resident', accel: 'ROCm', offload: '12/12 layers on GPU', layers: '12/12 layers · gfx1100' },
    'qwen2.5-coder:14b': { name: 'qwen2.5-coder:14b', short: 'qwen', size: '14b', vram: '10.1G', gpuVram: 12.6,
      ctx:'32k', ctxUsed:'6.1k', meta: 'Q5_K_M · on disk', accel: 'ROCm', offload: 'fits · 48/48 on GPU', layers: '48/48 layers · gfx1100' },
    'phi4:14b': { name: 'phi4:14b', short: 'phi4', size: '14b', vram: '8.4G', gpuVram: 10.9,
      ctx:'16k', ctxUsed:'3.8k', meta: 'Q4_0 · on disk', accel: 'ROCm', offload: 'fits · 40/40 on GPU', layers: '40/40 layers · gfx1100' },
    'deepseek-r1:32b': { name: 'deepseek-r1:32b', short: 'r1', size: '32b', vram: '19.8G', gpuVram: 23.1,
      ctx:'64k', ctxUsed:'11.5k', meta: 'Q4_K_M · on disk · reasoning', accel: 'CPU', offload: 'spills 9 layers → CPU', layers: '55/64 layers · 9 on CPU' }
  };

  const RESIDENT = ['llama3.1:70b', 'nomic-embed-text'];
  const AVAILABLE = ['qwen2.5-coder:14b', 'phi4:14b', 'deepseek-r1:32b'];

  // Detected at runtime by ollama; unavailable ones stay listed but inert so you
  // can see WHY a machine fell back to CPU.
  const BACKENDS = [
    { id: 'ROCm', note: 'AMD RX 7900 XTX · gfx1100 · ROCm 6.2', ok: true },
    { id: 'CUDA', note: 'No NVIDIA device detected', ok: false },
    { id: 'MLX', note: 'Apple silicon only', ok: false },
    { id: 'Vulkan', note: 'Available — slower than ROCm here', ok: true },
    { id: 'CPU', note: '16 threads · AVX-512', ok: true }
  ];

  const TOTAL_VRAM = 24;
  // VRAM other clients still hold when nothing is offloaded to the GPU.
  const IDLE_VRAM = 2.5;

  function resolve(backendId, modelId) {
    const m = MODELS[modelId] || MODELS['llama3.1:70b'];
    const forcedCpu = backendId === 'CPU';
    const accel = (forcedCpu || m.accel === 'CPU') ? 'CPU' : (backendId || 'ROCm');
    const total = (m.layers.split('/')[1] || '').split(' ')[0] || '0';
    const used = forcedCpu ? IDLE_VRAM : m.gpuVram;
    const layers = forcedCpu ? '0/' + total + ' on GPU · 16 threads' : m.layers;
    // Bare split for the settings chip, e.g. "81/81" or "0/81" under forced CPU.
    const gpuLayers = forcedCpu ? '0/' + total : m.layers.split(' ')[0];
    return {
      model: m, accel, isCpu: accel === 'CPU', layers, gpuLayers,
      ctxWindow: m.ctx, ctxLine: m.ctxUsed + ' / ' + m.ctx + ' ctx',
      device: accel === 'CPU' ? 'Ryzen 9 7950X' : 'RX 7900 XTX',
      vramUsed: used,
      vramPct: Math.round(used / TOTAL_VRAM * 100),
      vramLine: forcedCpu
        ? 'VRAM ' + IDLE_VRAM.toFixed(1) + ' / ' + TOTAL_VRAM.toFixed(1) + ' GiB · 41.2 / 64 GiB RAM'
        : 'VRAM ' + used.toFixed(1) + ' / ' + TOTAL_VRAM.toFixed(1) + ' GiB',
      tip: 'Ollama — ' + m.name + ' · ' + accel + ' · ' + layers
    };
  }

  window.TezcaLlm = { MODELS, RESIDENT, AVAILABLE, BACKENDS, resolve };
})();
