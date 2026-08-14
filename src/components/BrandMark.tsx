interface BrandMarkProps {
  size?: number;
}

// ContractorCRM mark: three descending columns with two connector stubs —
// a pipeline with work moving down it (geometry from docs/design/DESIGN.md §6).
export function BrandMark({ size = 44 }: BrandMarkProps) {
  return (
    <span className="brand-mark" style={{ width: size, height: size }} aria-hidden="true">
      <svg viewBox="0 0 32 32">
        <rect x="3" y="6" width="6" height="22" className="brand-mark__paper" />
        <rect x="12" y="11" width="6" height="17" className="brand-mark__accent" />
        <rect x="21" y="17" width="6" height="11" className="brand-mark__paper" />
        <rect x="9" y="11" width="3" height="1.6" className="brand-mark__accent" />
        <rect x="18" y="17" width="3" height="1.6" className="brand-mark__accent" />
      </svg>
    </span>
  );
}
