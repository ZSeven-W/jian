// Thin CanvasKit boundary for jian-host-web. Rust owns DrawOp dispatch and the
// image registry; this module only manages CanvasKit handles and immediate ops.

let canvasKitPromise;

const ICON_PATHS = {
  'pen-tool': 'M12 19l7-7 3 3-7 7-3-3z M18 13l-1.5-7.5L2 2l3.5 14.5L13 18l5-5z M2 2l7.586 7.586 M13 11a2 2 0 1 1-4 0a2 2 0 1 1 4 0Z',
  mail: 'M4 4h16a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2Z M22 7l-10 5L2 7',
  lock: 'M5 11h14a2 2 0 0 1 2 2v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2Z M7 11V7a5 5 0 0 1 10 0v4',
  'eye-off': 'M9.88 9.88a3 3 0 1 0 4.24 4.24 M10.73 5.08A10.43 10.43 0 0 1 12 5c7 0 10 7 10 7a13.16 13.16 0 0 1-1.67 2.68 M6.61 6.61A13.526 13.526 0 0 0 2 12s3 7 10 7a9.74 9.74 0 0 0 5.39-1.61 M2 2L22 22',
  chrome: 'M22 12a10 10 0 1 1-20 0a10 10 0 1 1 20 0Z M16 12a4 4 0 1 1-8 0a4 4 0 1 1 8 0Z M21.17 8H12 M3.95 6.06L8.54 14 M10.88 21.94L15.46 14',
  smartphone: 'M7 2h10a2 2 0 0 1 2 2v16a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2Z M12 18h.01',
};

function directory(base) {
  const value = String(base || '/canvaskit/');
  return value.endsWith('/') ? value : value + '/';
}

function loadScript(url) {
  return new Promise((resolve, reject) => {
    if (globalThis.CanvasKitInit) return resolve();
    const script = document.createElement('script');
    script.src = url;
    script.onload = resolve;
    script.onerror = () => reject(new Error(`failed to load CanvasKit from ${url}`));
    document.head.appendChild(script);
  });
}

export function jianCkInit(assetBase) {
  if (!canvasKitPromise) {
    const base = directory(assetBase);
    canvasKitPromise = loadScript(base + 'canvaskit.js')
      .then(() => {
        const options = { locateFile: (file) => base + file };
        if (globalThis.__jianCanvasKitWasmBinary) {
          options.wasmBinary = globalThis.__jianCanvasKitWasmBinary;
        }
        return globalThis.CanvasKitInit(options);
      });
  }
  return canvasKitPromise.then((ck) => new Runtime(ck));
}

function channels(packed) {
  const value = Number(packed) >>> 0;
  return [
    ((value >>> 24) & 255) / 255,
    ((value >>> 16) & 255) / 255,
    ((value >>> 8) & 255) / 255,
    (value & 255) / 255,
  ];
}

class Runtime {
  constructor(ck) {
    this.ck = ck;
    this.fonts = new Map();
    this.fontMgr = null;
    this.coverage = new Map();
    this.disposed = false;
  }

  registeredFontCount() { return this.fonts.size; }

  dispose() {
    if (this.disposed) return;
    this.disposed = true;
    if (this.fontMgr && this.fontMgr.delete) this.fontMgr.delete();
    this.fontMgr = null;
    for (const entry of this.fonts.values()) {
      if (entry.typeface && entry.typeface.delete) entry.typeface.delete();
    }
    this.fonts.clear();
    this.coverage.clear();
    this.ck = null;
  }

  makeSurface(canvas, width, height) {
    canvas.width = Math.max(1, width);
    canvas.height = Math.max(1, height);
    const surface = this.ck.MakeWebGLCanvasSurface(canvas)
      || (this.ck.MakeSWCanvasSurface && this.ck.MakeSWCanvasSurface(canvas));
    if (!surface) throw new Error('CanvasKit failed to make a canvas surface');
    return new Surface(this, surface);
  }

  decodeImage(bytes) {
    const owned = new Uint8Array(bytes).slice();
    const image = this.ck.MakeImageFromEncoded(owned);
    if (!image) throw new Error('CanvasKit rejected encoded image bytes');
    return image;
  }

  deleteImage(image) { if (image && image.delete) image.delete(); }

  registerFont(alias, actualFamily, bytes) {
    const key = String(alias || actualFamily).trim().toLowerCase();
    if (!key) return false;
    const owned = new Uint8Array(bytes).slice();
    const typeface = this.ck.Typeface.MakeFreeTypeFaceFromData(owned.buffer.slice(0));
    if (!typeface) return false;
    const old = this.fonts.get(key);
    if (this.fontMgr) this.fontMgr.delete();
    this.fontMgr = null;
    if (old && old.typeface) old.typeface.delete();
    this.fonts.set(key, { key, alias: String(alias), actual: String(actualFamily), bytes: owned, typeface });
    this.coverage.clear();
    this.fontMgr = this.ck.FontMgr.FromData(...Array.from(
      this.fonts.values(),
      (entry) => entry.bytes.buffer.slice(entry.bytes.byteOffset, entry.bytes.byteOffset + entry.bytes.byteLength),
    ));
    return Boolean(this.fontMgr);
  }

  entryFor(alias) {
    const first = String(alias || '').split(',')[0].trim().replace(/^['"]|['"]$/g, '').toLowerCase();
    return this.fonts.get(first) || null;
  }

  covers(entry, text) {
    if (!entry || !text) return false;
    const key = entry.key + '\n' + text;
    if (this.coverage.has(key)) return this.coverage.get(key);
    const font = new this.ck.Font(entry.typeface, 16);
    const ids = font.getGlyphIDs(text);
    font.delete();
    const result = ids.length > 0 && ids.every((id) => id !== 0);
    this.coverage.set(key, result);
    return result;
  }

  selectEntry(alias, character) {
    const primary = this.entryFor(alias);
    if (this.covers(primary, character)) return primary;
    for (const entry of this.fonts.values()) {
      if (entry !== primary && this.covers(entry, character)) return entry;
    }
    return null;
  }

  coversText(alias, text) {
    for (const character of String(text || '')) {
      if (!this.selectEntry(alias, character)) return false;
    }
    return true;
  }

  makeParagraph(texts, families, sizes, weights, italics, spacing, maxWidth, lineHeight, colors = []) {
    const manager = this.fontMgr || (this.ck.FontMgr.RefDefault && this.ck.FontMgr.RefDefault());
    if (!manager || !this.ck.ParagraphBuilder) return null;
    const allFamilies = Array.from(this.fonts.values(), (entry) => entry.actual);
    const firstSize = Number(sizes[0] || 16);
    const paragraphStyle = this.ck.ParagraphStyle({
      textStyle: { fontFamilies: allFamilies, fontSize: firstSize },
      maxLines: 10000,
    });
    const builder = this.ck.ParagraphBuilder.Make(paragraphStyle, manager);
    for (let run = 0; run < texts.length; run++) {
      const text = String(texts[run] || '');
      let current = null, segment = '';
      const flush = () => {
        if (!segment) return;
        const requested = this.entryFor(families[run]);
        const fontFamilies = current
          ? [current.actual, ...allFamilies.filter((name) => name !== current.actual)]
          : requested
            ? [requested.actual, ...allFamilies.filter((name) => name !== requested.actual)]
            : allFamilies;
        builder.pushStyle(this.ck.TextStyle({
          fontFamilies,
          fontSize: Number(sizes[run] || 16),
          fontStyle: { weight: Number(weights[run] || 400), slant: italics[run] ? 1 : 0 },
          letterSpacing: Number(spacing[run] || 0),
          heightMultiplier: lineHeight > 0 ? lineHeight : 1.3,
          color: colors[run] === undefined ? this.ck.BLACK : this.ck.Color4f(...channels(colors[run])),
        }));
        builder.addText(segment);
        builder.pop();
        segment = '';
      };
      for (const character of text) {
        const selected = this.selectEntry(families[run], character);
        if (selected !== current && segment) flush();
        current = selected;
        segment += character;
      }
      flush();
    }
    const paragraph = builder.build();
    builder.delete();
    paragraph.layout(maxWidth > 0 ? maxWidth : 1000000);
    return paragraph;
  }

  measureParagraph(texts, families, sizes, weights, italics, spacing, maxWidth, lineHeight) {
    const paragraph = this.makeParagraph(texts, families, sizes, weights, italics, spacing, maxWidth, lineHeight);
    if (!paragraph) return [0, 0, 0, 0];
    const metrics = paragraph.getLineMetrics ? paragraph.getLineMetrics() : [];
    const width = paragraph.getLongestLine ? paragraph.getLongestLine() : paragraph.getMaxIntrinsicWidth();
    const height = paragraph.getHeight();
    const baseline = metrics.length ? Number(metrics[0].baseline || sizes[0] * 0.8) : Number(sizes[0] || 0) * 0.8;
    const result = [width, height, Math.max(1, metrics.length || 1), baseline];
    paragraph.delete();
    return result;
  }
}

class Surface {
  constructor(runtime, surface) {
    this.runtime = runtime;
    this.ck = runtime.ck;
    this.surface = surface;
    this.canvas = surface.getCanvas();
    this.fill = new this.ck.Paint();
    this.stroke = new this.ck.Paint();
    this.fill.setAntiAlias(true);
    this.stroke.setAntiAlias(true);
    this.stroke.setStyle(this.ck.PaintStyle.Stroke);
    this.stroke.setStrokeCap(this.ck.StrokeCap.Round);
    this.stroke.setStrokeJoin(this.ck.StrokeJoin.Round);
    this.textWidth = 0;
  }

  color(value, opacity = 1) {
    const [r, g, b, a] = channels(value);
    return this.ck.Color4f(r, g, b, a * Math.max(0, Math.min(1, opacity)));
  }

  fillPaint(value, opacity) {
    this.fill.setStyle(this.ck.PaintStyle.Fill);
    this.fill.setShader(null);
    this.fill.setMaskFilter(null);
    this.fill.setColor(this.color(value, opacity));
    return this.fill;
  }

  strokePaint(value, width, opacity = 1) {
    this.stroke.setStyle(this.ck.PaintStyle.Stroke);
    this.stroke.setShader(null);
    this.stroke.setMaskFilter(null);
    this.stroke.setStrokeWidth(Math.max(0, width));
    this.stroke.setColor(this.color(value, opacity));
    return this.stroke;
  }

  rect(v) { return this.ck.LTRBRect(v[0], v[1], v[0] + v[2], v[1] + v[3]); }

  rrect(rect, radii) {
    const path = new this.ck.Path();
    const x = rect[0], y = rect[1], w = rect[2], h = rect[3];
    const r = Array.from(radii, (n) => Math.max(0, Number(n)));
    if (path.addRoundRect) {
      path.addRoundRect(this.ck.LTRBRect(x, y, x + w, y + h),
        [r[0], r[0], r[1], r[1], r[2], r[2], r[3], r[3]], true);
    } else {
      path.addRRect(this.ck.RRectXY(this.ck.LTRBRect(x, y, x + w, y + h), r[0], r[0]), true);
    }
    return path;
  }

  beginFrame(clear, dpr) {
    while (this.canvas.getSaveCount() > 1) this.canvas.restore();
    this.canvas.clear(this.color(clear));
    this.canvas.save();
    if (dpr !== 1) this.canvas.scale(dpr, dpr);
  }

  endFrame() {
    while (this.canvas.getSaveCount() > 1) this.canvas.restore();
    this.surface.flush();
  }

  pushClip(x, y, width, height) {
    this.canvas.save();
    this.canvas.clipRect(this.ck.LTRBRect(x, y, x + width, y + height), this.ck.ClipOp.Intersect, true);
  }

  pushTransform(m) {
    this.canvas.save();
    this.canvas.concat([m[0], m[2], m[4], m[1], m[3], m[5], 0, 0, 1]);
  }

  pop() { if (this.canvas.getSaveCount() > 1) this.canvas.restore(); }

  pushLayer(bounds, filterKind, filter) {
    let paint = null;
    let imageFilter = null;
    if (filterKind === 1 && this.ck.ImageFilter.MakeBlur) {
      imageFilter = this.ck.ImageFilter.MakeBlur(filter[0], filter[0], this.ck.TileMode.Decal, null);
    } else if (filterKind === 2 && this.ck.ImageFilter.MakeDropShadow) {
      imageFilter = this.ck.ImageFilter.MakeDropShadow(
        filter[0], filter[1], filter[2], filter[2],
        this.ck.Color4f(filter[4], filter[5], filter[6], filter[7]), null,
      );
    }
    if (imageFilter) {
      paint = new this.ck.Paint();
      paint.setImageFilter(imageFilter);
    }
    this.canvas.saveLayer(paint, this.rect(bounds));
    if (paint) paint.delete();
    if (imageFilter) imageFilter.delete();
  }

  drawRect(rect, fill, stroke, width, opacity) {
    const box = this.rect(rect);
    if (fill >= 0) this.canvas.drawRect(box, this.fillPaint(fill, opacity));
    if (stroke >= 0 && width > 0) this.canvas.drawRect(box, this.strokePaint(stroke, width, opacity));
  }

  drawRoundedRect(rect, radii, fill, stroke, width, opacity) {
    const path = this.rrect(rect, radii);
    if (fill >= 0) this.canvas.drawPath(path, this.fillPaint(fill, opacity));
    if (stroke >= 0 && width > 0) this.canvas.drawPath(path, this.strokePaint(stroke, width, opacity));
    path.delete();
  }

  drawPath(commands, fill, stroke, width, opacity) {
    const path = new this.ck.Path();
    for (let i = 0; i < commands.length;) {
      const op = commands[i++];
      if (op === 0) path.moveTo(commands[i++], commands[i++]);
      else if (op === 1) path.lineTo(commands[i++], commands[i++]);
      else if (op === 2) path.quadTo(commands[i++], commands[i++], commands[i++], commands[i++]);
      else if (op === 3) path.cubicTo(commands[i++], commands[i++], commands[i++], commands[i++], commands[i++], commands[i++]);
      else if (op === 4) path.close();
      else break;
    }
    if (fill >= 0) this.canvas.drawPath(path, this.fillPaint(fill, opacity));
    if (stroke >= 0 && width > 0) this.canvas.drawPath(path, this.strokePaint(stroke, width, opacity));
    path.delete();
  }

  drawImage(image, rect, opacity) {
    const paint = this.fillPaint(0xffffffff, opacity);
    this.canvas.drawImageRect(image, this.ck.XYWHRect(0, 0, image.width(), image.height()), this.rect(rect), paint);
  }

  drawText(text, family, rect, size, weight, color, align, lineHeight) {
    const paragraph = this.runtime.makeParagraph(
      [text], [family], [size], [weight], [0], [0], rect[2] > 0 ? rect[2] : -1, lineHeight, [color],
    );
    if (!paragraph) return;
    const width = paragraph.getLongestLine ? paragraph.getLongestLine() : paragraph.getMaxIntrinsicWidth();
    const available = Math.max(0, rect[2]);
    const x = rect[0] + (align === 1 ? Math.max(0, available - width) / 2 : align === 2 ? Math.max(0, available - width) : 0);
    this.canvas.drawParagraph(paragraph, x, rect[1]);
    this.textWidth = width;
    paragraph.delete();
  }

  drawRichText(texts, families, sizes, weights, italics, spacing, colors, rect, align, lineHeight) {
    const paragraph = this.runtime.makeParagraph(
      texts, families, sizes, weights, italics, spacing,
      rect[2] > 0 ? rect[2] : -1, lineHeight, colors,
    );
    if (!paragraph) return;
    const width = paragraph.getLongestLine ? paragraph.getLongestLine() : paragraph.getMaxIntrinsicWidth();
    const available = Math.max(0, rect[2]);
    const x = rect[0] + (align === 1 ? Math.max(0, available - width) / 2 : align === 2 ? Math.max(0, available - width) : 0);
    this.canvas.drawParagraph(paragraph, x, rect[1]);
    this.textWidth = width;
    paragraph.delete();
  }

  stops(values, opacity) {
    const colors = [], positions = [];
    for (let i = 0; i + 4 < values.length; i += 5) {
      positions.push(values[i]);
      colors.push(this.ck.Color4f(values[i + 1], values[i + 2], values[i + 3], values[i + 4] * opacity));
    }
    return { colors, positions };
  }

  gradientPath(rect, radii, shader, stroke, strokeWidth, opacity = 1) {
    const path = this.rrect(rect, radii);
    if (shader) {
      const paint = new this.ck.Paint();
      paint.setAntiAlias(true);
      paint.setShader(shader);
      paint.setAlphaf(Math.max(0, Math.min(1, opacity)));
      this.canvas.drawPath(path, paint);
      paint.delete();
      shader.delete();
    }
    if (stroke >= 0 && strokeWidth > 0) this.canvas.drawPath(path, this.strokePaint(stroke, strokeWidth));
    path.delete();
  }

  drawLinearGradient(rect, radii, angle, values, opacity, stroke, strokeWidth) {
    const { colors, positions } = this.stops(values, opacity);
    if (!colors.length) return;
    const radians = angle * Math.PI / 180;
    const cx = rect[0] + rect[2] / 2, cy = rect[1] + rect[3] / 2;
    const dx = Math.cos(radians) * rect[2] / 2, dy = Math.sin(radians) * rect[3] / 2;
    const shader = this.ck.Shader.MakeLinearGradient([cx - dx, cy - dy], [cx + dx, cy + dy], colors, positions, this.ck.TileMode.Clamp);
    this.gradientPath(rect, radii, shader, stroke, strokeWidth);
  }

  drawRadialGradient(rect, radii, cx, cy, radius, values, opacity, stroke, strokeWidth) {
    const stopData = this.stops(values, opacity);
    if (!stopData.colors.length) return;
    const center = [rect[0] + rect[2] * cx, rect[1] + rect[3] * cy];
    const pxRadius = Math.max(rect[2], rect[3]) * radius;
    const shader = this.ck.Shader.MakeRadialGradient(center, Math.max(0.01, pxRadius), stopData.colors, stopData.positions, this.ck.TileMode.Clamp);
    this.gradientPath(rect, radii, shader, stroke, strokeWidth);
  }

  drawMeshGradient(rect, radii, rows, cols, colors, opacity, stroke, strokeWidth) {
    if (rows < 2 || cols < 2 || colors.length !== rows * cols || !this.ck.MakeVertices) {
      if (colors.length) this.drawRoundedRect(rect, radii, colors[0], stroke, strokeWidth, opacity);
      return;
    }
    const positions = [], vertexColors = [], indices = [];
    for (let row = 0; row < rows; row++) for (let col = 0; col < cols; col++) {
      positions.push(rect[0] + rect[2] * col / (cols - 1), rect[1] + rect[3] * row / (rows - 1));
      const rgba = channels(colors[row * cols + col]);
      vertexColors.push(this.ck.Color4f(rgba[0], rgba[1], rgba[2], rgba[3] * opacity));
    }
    for (let row = 0; row + 1 < rows; row++) for (let col = 0; col + 1 < cols; col++) {
      const a = row * cols + col, b = a + 1, c = a + cols, d = c + 1;
      indices.push(a, b, c, b, d, c);
    }
    const vertices = this.ck.MakeVertices(this.ck.VertexMode.Triangles, positions, null, vertexColors, indices, true);
    const path = this.rrect(rect, radii);
    this.canvas.save();
    this.canvas.clipPath(path, this.ck.ClipOp.Intersect, true);
    this.canvas.drawVertices(vertices, this.ck.BlendMode.SrcOver, this.fillPaint(0xffffffff, 1));
    this.canvas.restore();
    if (stroke >= 0 && strokeWidth > 0) this.canvas.drawPath(path, this.strokePaint(stroke, strokeWidth));
    vertices.delete(); path.delete();
  }

  drawShader(rect, radii, sksl, uniformNames, uniforms, uniformArities, opacity, fallback, stroke, strokeWidth) {
    let shader = null, effect = null;
    try {
      effect = this.ck.RuntimeEffect.Make(sksl);
      if (effect) {
        const data = new Float32Array(effect.getUniformFloatCount());
        const reflected = new Map();
        for (let index = 0; index < effect.getUniformCount(); index++) {
          reflected.set(effect.getUniformName(index), effect.getUniform(index));
        }
        let cursor = 0;
        for (let index = 0; index < uniformNames.length; index++) {
          const arity = Number(uniformArities[index] || 0);
          const values = uniforms.slice(cursor, cursor + arity);
          cursor += arity;
          const info = reflected.get(String(uniformNames[index]));
          const expected = info ? Number(info.columns) * Number(info.rows) : 0;
          if (!info || info.isInteger || expected !== arity) continue;
          for (let value = 0; value < arity; value++) data[Number(info.slot) + value] = values[value];
        }
        shader = effect.makeShader(data);
      }
    } catch (_) { shader = null; }
    if (!shader) {
      if (effect) effect.delete();
      this.drawRoundedRect(rect, radii, fallback, stroke, strokeWidth, opacity);
      return;
    }
    this.gradientPath(rect, radii, shader, stroke, strokeWidth, opacity);
    effect.delete();
  }

  drawShadow(rect, radii, shadow) {
    const path = this.rrect([rect[0] + shadow[0], rect[1] + shadow[1], rect[2], rect[3]], radii);
    const paint = new this.ck.Paint();
    paint.setAntiAlias(true);
    paint.setColor(this.ck.Color4f(shadow[4], shadow[5], shadow[6], shadow[7]));
    if (this.ck.MaskFilter.MakeBlur) {
      const blur = this.ck.MaskFilter.MakeBlur(this.ck.BlurStyle.Normal, Math.max(0, shadow[2]), true);
      paint.setMaskFilter(blur);
      this.canvas.drawPath(path, paint);
      blur.delete();
    } else this.canvas.drawPath(path, paint);
    paint.delete(); path.delete();
  }

  drawIcon(rect, name, _family, color) {
    const data = ICON_PATHS[name];
    if (!data) {
      this.canvas.drawRect(this.rect(rect), this.fillPaint(color, 1));
      return;
    }
    const path = this.ck.Path.MakeFromSVGString(data);
    if (!path) return;
    path.transform(this.ck.Matrix.multiply(
      this.ck.Matrix.translated(rect[0], rect[1]),
      this.ck.Matrix.scaled(rect[2] / 24, rect[3] / 24),
    ));
    this.canvas.drawPath(path, this.strokePaint(color, 2 * Math.max(0.1, Math.min(rect[2], rect[3]) / 24)));
    path.delete();
  }

  pixels(x, y, width, height) {
    const image = this.surface.makeImageSnapshot();
    let bytes = null;
    try {
      bytes = image.readPixels(x, y, {
        width, height,
        colorType: this.ck.ColorType.RGBA_8888,
        alphaType: this.ck.AlphaType.Unpremul,
        colorSpace: this.ck.ColorSpace.SRGB,
      });
    } finally { image.delete(); }
    return bytes ? new Uint8Array(bytes) : new Uint8Array();
  }

  readPixel(x, y) { return this.pixels(x, y, 1, 1); }

  regionHasInk(x, y, width, height) {
    const bytes = this.pixels(x, y, width, height);
    for (let i = 0; i + 3 < bytes.length; i += 4) {
      if (bytes[i] < 245 || bytes[i + 1] < 245 || bytes[i + 2] < 245) return true;
    }
    return false;
  }

  lastTextWidth() { return this.textWidth; }

  dispose() {
    if (!this.surface) return;
    this.fill.delete(); this.stroke.delete(); this.surface.delete();
    this.surface = null; this.canvas = null;
  }
}
