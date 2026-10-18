// Layout blocks of the showcase page: a section (h2), a specimen (h3) and a
// labelled row of examples.

import type { ReactNode } from 'react';
import styles from './Section.module.css';

interface SectionProps {
  id: string;
  title: string;
  description?: ReactNode;
  children: ReactNode;
}

export function Section({ id, title, description, children }: SectionProps) {
  const titleId = `${id}-title`;
  return (
    <section id={id} aria-labelledby={titleId} className={styles.section}>
      <header className={styles.header}>
        <h2 id={titleId} className={styles.title}>
          {title}
        </h2>
        {description === undefined ? null : <p className={styles.description}>{description}</p>}
      </header>
      {children}
    </section>
  );
}

interface SpecimenProps {
  id: string;
  title: string;
  description?: ReactNode;
  children: ReactNode;
}

/** One component or topic inside a section. */
export function Specimen({ id, title, description, children }: SpecimenProps) {
  const titleId = `${id}-title`;
  return (
    <article id={id} aria-labelledby={titleId} className={styles.specimen}>
      <header className={styles.header}>
        <h3 id={titleId} className={styles.specimenTitle}>
          {title}
        </h3>
        {description === undefined ? null : <p className={styles.description}>{description}</p>}
      </header>
      {children}
    </article>
  );
}

interface RowProps {
  label: ReactNode;
  children: ReactNode;
}

/** A caption on the left and examples on the right; wraps on narrow screens. */
export function Row({ label, children }: RowProps) {
  return (
    <div className={styles.row}>
      <div className={styles.rowLabel}>{label}</div>
      <div className={styles.rowItems}>{children}</div>
    </div>
  );
}
