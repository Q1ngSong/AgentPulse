// 小型页面动效与本地提醒演示；不连接 App、不收集数据。
const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)');
if ('IntersectionObserver' in window && !reducedMotion.matches) {
  const observer = new IntersectionObserver(entries => {
    for (const entry of entries) if (entry.isIntersecting) {
      entry.target.classList.add('is-visible');
      observer.unobserve(entry.target);
    }
  }, { threshold: 0.12 });
  for (const section of document.querySelectorAll('.reveal')) {
    section.classList.add('will-reveal');
    observer.observe(section);
  }
}

const notice = document.querySelector('#demo-notice');
if (notice) {
  let expiry;
  const title = document.querySelector('#notice-title');
  const body = document.querySelector('#notice-body');
  const result = document.querySelector('#terminal-result');
  const status = document.querySelector('#demo-status');
  const controls = document.querySelectorAll('[data-demo]');
  function dismiss(message) {
    clearTimeout(expiry);
    notice.hidden = true;
    status.textContent = message;
  }
  function show(kind) {
    clearTimeout(expiry);
    for (const button of controls) button.setAttribute('aria-pressed', String(button.dataset.demo === kind));
    const permission = kind === 'permission';
    title.textContent = permission ? 'Codex · 请求权限' : 'Codex · 任务完成';
    body.textContent = permission ? '需要你的确认，请回到来源工具处理。' : '功能已实现，测试通过。回来看看吧。';
    result.textContent = permission ? '→ 等待授权，准备执行下一步。' : '✓ 任务已完成，等待你查看。';
    notice.hidden = false;
    // 重启每次提醒的 5 秒进度；用户可随时关闭或返回来源。
    for (const animation of notice.getAnimations({ subtree: true })) { animation.cancel(); animation.play(); }
    status.textContent = '交互示意 · 提醒将在 5 秒后自动收起';
    expiry = setTimeout(() => dismiss('提醒已自动收起 · 点击上方按钮再次体验'), 5000);
  }
  for (const button of controls) button.addEventListener('click', () => show(button.dataset.demo));
  document.querySelector('#notice-close').addEventListener('click', () => dismiss('已关闭这条演示提醒'));
  document.querySelector('#notice-activate').addEventListener('click', () => {
    dismiss('已返回消息来源 · 此处为网页演示');
    document.querySelector('.terminal').animate([{ outlineColor: 'transparent' }, { outlineColor: '#3770ee' }, { outlineColor: 'transparent' }], { duration: reducedMotion.matches ? 0 : 850 });
  });
  if (!reducedMotion.matches) expiry = setTimeout(() => show('complete'), 1200);
}
