/// <reference types="vite/client" />

// CSS modules: `vite/client` declares `*.module.css` imports as a default
// export of type `CSSModuleClasses`, a read-only map from the class name in
// the stylesheet to the generated class name:
//
//   import styles from './Button.module.css';
//   <button className={styles.primary} />
//
// Plain `*.css` imports (for example `./theme/tokens.css`) are side-effect
// imports, also declared by `vite/client`.
