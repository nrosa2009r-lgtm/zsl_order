/* ZSL Order — lekkie wykresy na canvasie (bez zewnętrznych bibliotek). */
(function () {
    "use strict";

    const DEFAULTS = {
        top: 16,
        right: 14,
        bottom: 34,
        left: 44,
    };

    function setup(canvas) {
        const dpr = window.devicePixelRatio || 1;
        const width = Math.max(280, Math.floor(canvas.clientWidth || 620));
        const height = Math.max(180, Math.floor(canvas.clientHeight || 260));

        canvas.width = Math.round(width * dpr);
        canvas.height = Math.round(height * dpr);

        const ctx = canvas.getContext("2d");
        ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
        ctx.clearRect(0, 0, width, height);
        return { ctx: ctx, width: width, height: height };
    }

    function readColors() {
        const styles = window.getComputedStyle(document.documentElement);
        function read(name, fallback) {
            const value = (styles.getPropertyValue(name) || "").trim();
            return value || fallback;
        }
        return {
            primary: read("--primary", "#227b88"),
            primaryDark: read("--primary-dark", "#175b65"),
            accent: read("--accent", "#f0b400"),
            danger: read("--danger", "#b42318"),
            success: read("--success", "#18794e"),
            muted: read("--muted", "#5b6f7c"),
            line: read("--line", "#d8e5e8"),
            text: read("--text", "#172b3a"),
        };
    }

    function withAlpha(hex, alpha) {
        if (/^#[0-9a-f]{6}$/i.test(hex)) {
            const value = Number.parseInt(hex.slice(1), 16);
            const red = (value >> 16) & 255;
            const green = (value >> 8) & 255;
            const blue = value & 255;
            return "rgba(" + red + "," + green + "," + blue + "," + alpha + ")";
        }
        return hex;
    }

    function niceMax(value) {
        if (!Number.isFinite(value) || value <= 0) return 1;
        const power = Math.pow(10, Math.floor(Math.log10(value)));
        const scaled = value / power;
        let nice;
        if (scaled <= 1) nice = 1;
        else if (scaled <= 2) nice = 2;
        else if (scaled <= 5) nice = 5;
        else nice = 10;
        return nice * power;
    }

    function numberFormat(value) {
        return new Intl.NumberFormat("pl-PL", { maximumFractionDigits: 0 }).format(value);
    }

    function drawGrid(ctx, colors, width, height, max, padding, formatter) {
        const plotHeight = height - padding.top - padding.bottom;
        const plotWidth = width - padding.left - padding.right;
        const ticks = 4;

        ctx.font = "11px system-ui, -apple-system, Segoe UI, Roboto, sans-serif";
        ctx.textAlign = "right";
        ctx.textBaseline = "middle";

        for (let index = 0; index <= ticks; index += 1) {
            const ratio = index / ticks;
            const y = padding.top + plotHeight - ratio * plotHeight;
            ctx.strokeStyle = colors.line;
            ctx.lineWidth = 1;
            ctx.beginPath();
            ctx.moveTo(padding.left, y);
            ctx.lineTo(padding.left + plotWidth, y);
            ctx.stroke();

            ctx.fillStyle = colors.muted;
            ctx.fillText(formatter(max * ratio), padding.left - 8, y);
        }
    }

    function drawXLabels(ctx, colors, labels, width, height, padding, formatter) {
        const plotWidth = width - padding.left - padding.right;
        const count = labels.length;
        if (!count) return;

        const step = Math.max(1, Math.ceil(count / 8));
        ctx.font = "11px system-ui, -apple-system, Segoe UI, Roboto, sans-serif";
        ctx.fillStyle = colors.muted;
        ctx.textAlign = "center";
        ctx.textBaseline = "top";

        for (let index = 0; index < count; index += step) {
            const x = padding.left + (count <= 1 ? plotWidth / 2 : (index / (count - 1)) * plotWidth);
            ctx.fillText(formatter(labels[index]), x, height - padding.bottom + 10);
        }
    }

    function line(canvas, labels, values, options) {
        if (!canvas) return;
        options = options || {};
        const colors = readColors();
        const prepared = setup(canvas);
        const ctx = prepared.ctx;
        const width = prepared.width;
        const height = prepared.height;
        const padding = Object.assign({}, DEFAULTS, options.padding || {});
        const plotWidth = width - padding.left - padding.right;
        const plotHeight = height - padding.top - padding.bottom;
        const max = niceMax(Math.max(1, Math.max.apply(null, values)));
        const color = options.color || colors.primary;
        const labelFormatter = options.labelFormatter || function (label) { return label; };
        const valueFormatter = options.valueFormatter || numberFormat;

        const x = function (index) {
            if (values.length <= 1) return padding.left + plotWidth / 2;
            return padding.left + (index / (values.length - 1)) * plotWidth;
        };
        const y = function (value) {
            return padding.top + plotHeight - (value / max) * plotHeight;
        };

        drawGrid(ctx, colors, width, height, max, padding, valueFormatter);

        if (values.length) {
            const gradient = ctx.createLinearGradient(0, padding.top, 0, padding.top + plotHeight);
            gradient.addColorStop(0, withAlpha(color, 0.3));
            gradient.addColorStop(1, withAlpha(color, 0.02));

            ctx.beginPath();
            ctx.moveTo(x(0), y(values[0]));
            for (let index = 1; index < values.length; index += 1) {
                ctx.lineTo(x(index), y(values[index]));
            }
            ctx.lineTo(x(values.length - 1), padding.top + plotHeight);
            ctx.lineTo(x(0), padding.top + plotHeight);
            ctx.closePath();
            ctx.fillStyle = gradient;
            ctx.fill();

            ctx.beginPath();
            ctx.moveTo(x(0), y(values[0]));
            for (let index = 1; index < values.length; index += 1) {
                ctx.lineTo(x(index), y(values[index]));
            }
            ctx.strokeStyle = color;
            ctx.lineWidth = 2.5;
            ctx.lineJoin = "round";
            ctx.lineCap = "round";
            ctx.stroke();

            if (values.length <= 31) {
                ctx.fillStyle = color;
                for (let index = 0; index < values.length; index += 1) {
                    ctx.beginPath();
                    ctx.arc(x(index), y(values[index]), 2.6, 0, Math.PI * 2);
                    ctx.fill();
                }
            }
        }

        drawXLabels(ctx, colors, labels, width, height, padding, labelFormatter);
    }

    function bars(canvas, labels, values, options) {
        if (!canvas) return;
        options = options || {};
        const colors = readColors();
        const prepared = setup(canvas);
        const ctx = prepared.ctx;
        const width = prepared.width;
        const height = prepared.height;
        const padding = Object.assign({}, DEFAULTS, options.padding || {});
        const plotWidth = width - padding.left - padding.right;
        const plotHeight = height - padding.top - padding.bottom;
        const max = niceMax(Math.max(1, Math.max.apply(null, values)));
        const color = options.color || colors.accent;
        const labelFormatter = options.labelFormatter || function (label) { return label; };
        const valueFormatter = options.valueFormatter || numberFormat;

        drawGrid(ctx, colors, width, height, max, padding, valueFormatter);

        const slot = plotWidth / Math.max(values.length, 1);
        const barWidth = Math.max(2, Math.min(34, slot * 0.62));

        for (let index = 0; index < values.length; index += 1) {
            const value = values[index];
            const barHeight = Math.max(value > 0 ? 2 : 0, (value / max) * plotHeight);
            const x = padding.left + slot * index + (slot - barWidth) / 2;
            const y = padding.top + plotHeight - barHeight;
            const highlighted = options.highlightIndex === index;

            ctx.fillStyle = highlighted ? colors.primaryDark : color;
            ctx.beginPath();
            roundedRect(ctx, x, y, barWidth, barHeight, Math.min(4, barWidth / 2));
            ctx.fill();
        }

        drawXLabels(ctx, colors, labels, width, height, padding, labelFormatter);
    }

    function horizontal(canvas, entries, options) {
        if (!canvas) return;
        options = options || {};
        const colors = readColors();
        const prepared = setup(canvas);
        const ctx = prepared.ctx;
        const width = prepared.width;
        const height = prepared.height;

        const padding = { top: 14, right: 12, bottom: 12, left: 12 };
        const labelWidth = Math.min(190, Math.max(100, width * 0.38));
        const valueWidth = 62;
        const plotWidth = width - padding.left - padding.right - labelWidth - valueWidth;
        const plotHeight = height - padding.top - padding.bottom;
        const rowHeight = entries.length ? plotHeight / entries.length : plotHeight;
        const max = niceMax(Math.max(1, Math.max.apply(null, entries.map(function (entry) { return entry.value; }))));

        ctx.font = "12px system-ui, -apple-system, Segoe UI, Roboto, sans-serif";
        ctx.textBaseline = "middle";

        entries.forEach(function (entry, index) {
            const centerY = padding.top + rowHeight * index + rowHeight / 2;
            const barHeight = Math.min(18, rowHeight * 0.55);
            const barX = padding.left + labelWidth;
            const barY = centerY - barHeight / 2;
            const barWidth = max > 0 ? (entry.value / max) * plotWidth : 0;

            ctx.fillStyle = colors.muted;
            ctx.textAlign = "right";
            ctx.fillText(truncate(ctx, entry.label, labelWidth - 10), padding.left + labelWidth - 10, centerY);

            ctx.fillStyle = withAlpha(colors.line, 0.9);
            roundedRect(ctx, barX, barY, plotWidth, barHeight, 4);
            ctx.fill();

            ctx.fillStyle = entry.color || options.color || colors.primary;
            roundedRect(ctx, barX, barY, Math.max(entry.value > 0 ? 2 : 0, barWidth), barHeight, 4);
            ctx.fill();

            ctx.fillStyle = colors.text;
            ctx.textAlign = "left";
            ctx.fillText(String(entry.display === undefined ? entry.value : entry.display), barX + plotWidth + 8, centerY);
        });
    }

    function roundedRect(ctx, x, y, width, height, radius) {
        const safeRadius = Math.min(radius, Math.abs(width) / 2, Math.abs(height) / 2);
        ctx.beginPath();
        ctx.moveTo(x + safeRadius, y);
        ctx.lineTo(x + width - safeRadius, y);
        ctx.quadraticCurveTo(x + width, y, x + width, y + safeRadius);
        ctx.lineTo(x + width, y + height - safeRadius);
        ctx.quadraticCurveTo(x + width, y + height, x + width - safeRadius, y + height);
        ctx.lineTo(x + safeRadius, y + height);
        ctx.quadraticCurveTo(x, y + height, x, y + height - safeRadius);
        ctx.lineTo(x, y + safeRadius);
        ctx.quadraticCurveTo(x, y, x + safeRadius, y);
        ctx.closePath();
    }

    function truncate(ctx, text, maxWidth) {
        let value = String(text == null ? "" : text);
        if (ctx.measureText(value).width <= maxWidth) return value;
        while (value.length > 1 && ctx.measureText(value + "…").width > maxWidth) {
            value = value.slice(0, -1);
        }
        return value + "…";
    }

    window.Charts = {
        line: line,
        bars: bars,
        horizontal: horizontal,
    };
})();
