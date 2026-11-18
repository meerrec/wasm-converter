import { useEffect, useRef, useState, type ReactElement } from 'react';
import { OfficeDialog } from './OfficeDialog.js';

interface Stroke {
  color: string;
  width: number;
  points: { x: number; y: number }[];
}

function paint(canvas: HTMLCanvasElement, strokes: Stroke[]): void {
  const context = canvas.getContext('2d');
  if (!context) return;
  context.fillStyle = '#ffffff';
  context.fillRect(0, 0, canvas.width, canvas.height);
  context.lineCap = 'round';
  context.lineJoin = 'round';
  for (const stroke of strokes) {
    context.strokeStyle = stroke.color;
    context.lineWidth = stroke.width;
    context.beginPath();
    const first = stroke.points[0];
    if (!first) continue;
    context.moveTo(first.x, first.y);
    for (const point of stroke.points.slice(1)) context.lineTo(point.x, point.y);
    if (stroke.points.length === 1) context.lineTo(first.x + 0.01, first.y + 0.01);
    context.stroke();
  }
}

/** Перо/маркер/ластик ленты: рисунок уходит в документ настоящей PNG-картинкой. */
export function DrawingPad(props: {
  mode: 'pen' | 'marker' | 'eraser';
  onClose(): void;
  onInsert(bytes: ArrayBuffer, width: number, height: number): Promise<void>;
}): ReactElement {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const [strokes, setStrokes] = useState<Stroke[]>([]);
  const [active, setActive] = useState<Stroke | null>(null);
  const [busy, setBusy] = useState(false);
  const width = 720;
  const height = 360;

  useEffect(() => {
    if (canvasRef.current) paint(canvasRef.current, strokes);
  }, [strokes]);

  const point = (event: React.PointerEvent<HTMLCanvasElement>): { x: number; y: number } => {
    const rect = event.currentTarget.getBoundingClientRect();
    return {
      x: (event.clientX - rect.left) * (width / rect.width),
      y: (event.clientY - rect.top) * (height / rect.height),
    };
  };
  const shown = active ? [...strokes, active] : strokes;
  useEffect(() => {
    if (canvasRef.current) paint(canvasRef.current, shown);
  }, [active]);

  return (
    <OfficeDialog
      title={props.mode === 'pen' ? 'Перо' : props.mode === 'marker' ? 'Маркер' : 'Ластик'}
      onClose={props.onClose}
      actions={
        <>
          <button type="button" disabled={busy || strokes.length === 0} onClick={() => setStrokes((all) => all.slice(0, -1))}>Отменить штрих</button>
          <button type="button" disabled={busy || strokes.length === 0} onClick={() => setStrokes([])}>Очистить</button>
          <button
            type="button"
            className="ofc-primary"
            disabled={busy || strokes.length === 0}
            onClick={() => {
              const canvas = canvasRef.current;
              if (!canvas) return;
              setBusy(true);
              canvas.toBlob((blob) => {
                if (!blob) {
                  setBusy(false);
                  return;
                }
                void blob.arrayBuffer().then((bytes) => props.onInsert(bytes, width, height)).finally(() => setBusy(false));
              }, 'image/png');
            }}
          >Вставить рисунок</button>
        </>
      }
    >
      <canvas
        ref={canvasRef}
        className="ofc-drawing-pad"
        width={width}
        height={height}
        onPointerDown={(event) => {
          event.currentTarget.setPointerCapture(event.pointerId);
          setActive({
            color: props.mode === 'marker' ? 'rgba(255, 220, 0, 0.45)' : props.mode === 'eraser' ? '#ffffff' : '#17365d',
            width: props.mode === 'marker' ? 22 : props.mode === 'eraser' ? 36 : 3,
            points: [point(event)],
          });
        }}
        onPointerMove={(event) => {
          if (!active) return;
          const next = point(event);
          setActive((stroke) => stroke ? { ...stroke, points: [...stroke.points, next] } : null);
        }}
        onPointerUp={() => {
          if (active) setStrokes((all) => [...all, active]);
          setActive(null);
        }}
        onPointerCancel={() => setActive(null)}
      />
    </OfficeDialog>
  );
}
