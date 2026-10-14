import { useEffect, useId, useRef, useState } from "react";

export const readablePercent = (value: number) => Number(value.toPrecision(6));
export function VolumeControl({
  value,
  commit,
}: {
  value: number;
  commit(value: number): void;
}) {
  const id = useId();
  const displayed = readablePercent(value);
  const [local, setLocal] = useState(displayed);
  const [text, setText] = useState(String(displayed));
  const sent = useRef(displayed);
  const sliderEdited = useRef(false);
  const sliderValue = useRef(displayed);
  useEffect(() => {
    setLocal(displayed);
    setText(String(displayed));
    sent.current = displayed;
    sliderEdited.current = false;
    sliderValue.current = displayed;
  }, [displayed]);
  const send = (next: number) => {
    if (!Number.isFinite(next) || next < 0 || next > 100) return;
    if (next === sent.current) return;
    sent.current = next;
    commit(next);
  };
  const submit = () => {
    if (!text.trim()) return;
    const next = Number(text);
    if (Number.isFinite(next) && next >= 0 && next <= 100) {
      setLocal(next);
      send(next);
    }
  };
  const finishSlider = () => {
    if (sliderEdited.current) {
      sliderEdited.current = false;
      send(sliderValue.current);
    }
  };
  return (
    <div className="precise-volume">
      <label htmlFor={id}>
        모니터링 볼륨 <output>{local}%</output>
      </label>
      <input
        id={id}
        type="range"
        min="0"
        max="100"
        step="0.01"
        value={local}
        onChange={(event) => {
          const next = Number(event.target.value);
          setLocal(next);
          setText(String(next));
          sliderEdited.current = true;
          sliderValue.current = next;
        }}
        onPointerUp={finishSlider}
        onKeyUp={finishSlider}
        onBlur={finishSlider}
      />
      <label className="percentage-input">
        직접 입력
        <input
          aria-label="볼륨 백분율 직접 입력"
          type="number"
          min="0"
          max="100"
          step="any"
          value={text}
          onChange={(event) => setText(event.target.value)}
          onBlur={submit}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              submit();
            }
          }}
        />{" "}
        %
      </label>
      <small>
        0.5%, 0.05%처럼 입력할 수 있습니다. Enter 또는 입력 밖을 누르면
        적용됩니다.
      </small>
    </div>
  );
}
