// Tile color of a host or group: one of the 8 colors (design-system 3.1),
// graphite by default (UI 3.1, 3.4).

import { Check } from 'lucide-react';
import { TAG_COLORS } from '../../../components';
import type { TagColor } from '../../../ipc';
import { t } from '../../../i18n';
import styles from '../panels.module.css';

const swatchClass: Readonly<Record<TagColor, string>> = {
  graphite: styles.swatchGraphite,
  jade: styles.swatchJade,
  amber: styles.swatchAmber,
  coral: styles.swatchCoral,
  plum: styles.swatchPlum,
  moss: styles.swatchMoss,
  ocean: styles.swatchOcean,
  rose: styles.swatchRose,
};

export interface ColorFieldProps {
  value: TagColor;
  onChange: (color: TagColor) => void;
  /** Group test id; each button gets `<id>-<color>`. */
  'data-testid': string;
}

export function ColorField({ value, onChange, 'data-testid': testId }: ColorFieldProps) {
  const labelId = `${testId}-label`;
  return (
    <div className={styles.field}>
      <span id={labelId} className={styles.label}>
        {t('panels.color.label')}
      </span>
      <div role="group" aria-labelledby={labelId} className={styles.swatches} data-testid={testId}>
        {TAG_COLORS.map((color) => {
          const pressed = color === value;
          return (
            <button
              key={color}
              type="button"
              className={[styles.swatch, swatchClass[color]].join(' ')}
              aria-pressed={pressed}
              aria-label={t(`panels.color.${color}`)}
              title={t(`panels.color.${color}`)}
              onClick={() => onChange(color)}
              data-testid={`${testId}-${color}`}
            >
              {pressed ? <Check size={14} strokeWidth={2.5} aria-hidden="true" focusable="false" /> : null}
            </button>
          );
        })}
      </div>
    </div>
  );
}

export default ColorField;
