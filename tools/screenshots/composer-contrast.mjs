import { contrastRatio } from "../tests/lib/color-contrast.mjs";

// 模擬ブラウザーと実WebViewで同じ入力欄の可読性を検査する。
export async function composerContrast(page) {
  const colors = await page.locator("#input").evaluate(input => {
    const style = getComputedStyle(input);
    const placeholder = getComputedStyle(input, "::placeholder");
    return { background: style.backgroundColor, border: style.borderTopColor, text: style.color, placeholder: placeholder.color };
  });
  const parse = value => {
    if (!/^rgba?\(/.test(value)) throw new Error(`入力欄の色を解析できません: ${value}`);
    return value.match(/[\d.]+/g).map(Number);
  };
  const background = parse(colors.background);
  const againstBackground = value => {
    const color = parse(value);
    const alpha = color[3] ?? 1;
    const blended = color.slice(0, 3).map((channel, index) => channel * alpha + background[index] * (1 - alpha));
    return contrastRatio(blended, background.slice(0, 3));
  };
  return {
    alpha: background[3] ?? 1,
    border: againstBackground(colors.border),
    text: againstBackground(colors.text),
    placeholder: againstBackground(colors.placeholder),
  };
}

export function composerContrastIssues(result) {
  return [
    ...(result.alpha !== 1 ? ["入力欄の背景が透過しています"] : []),
    ...Object.entries({ border: 3, text: 4.5, placeholder: 4.5 })
      .filter(([key, minimum]) => result[key] < minimum)
      .map(([key, minimum]) => `${key}のコントラストが${minimum}未満です（${result[key].toFixed(2)}）`),
  ];
}
