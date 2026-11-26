import React from "react";

interface IconProps {
  className?: string;
}

/**
 * Sharp square-cornered folder icon ported 1:1 from `crates/muxlane-app/src/icons.rs` (FOLDER_ICON).
 */
export const ProjectFolderIcon: React.FC<IconProps> = ({ className = "h-3.5 w-3.5" }) => (
  <svg
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    strokeWidth="1.8"
    strokeLinecap="square"
    strokeLinejoin="miter"
    aria-hidden="true"
    className={className}
  >
    <path d="M3 5h6l2 2h10v12H3z" />
  </svg>
);

/**
 * Square-cornered Muxlane brand icon ported from `packaging/muxlane.svg`.
 */
export const MuxlaneLogoIcon: React.FC<IconProps> = ({ className = "h-5 w-5" }) => (
  <svg
    viewBox="0 0 512 512"
    fill="none"
    aria-hidden="true"
    className={className}
  >
    <rect x="24" y="24" width="464" height="464" fill="#181825" stroke="#45475a" strokeWidth="14" />
    <rect x="24" y="24" width="464" height="84" fill="#11111b" />
    <rect x="64" y="54" width="24" height="24" fill="#f38ba8" />
    <rect x="102" y="54" width="24" height="24" fill="#f9e2af" />
    <rect x="140" y="54" width="24" height="24" fill="#a6e3a1" />
    <path
      d="M92 184 L176 254 L92 324"
      stroke="#89b4fa"
      strokeWidth="32"
      strokeLinecap="square"
      strokeLinejoin="miter"
    />
    <rect x="212" y="300" width="96" height="28" fill="#a6e3a1" />
    <rect x="262" y="160" width="166" height="104" fill="#11111b" stroke="#45475a" strokeWidth="10" />
    <rect x="344" y="284" width="84" height="104" fill="#11111b" stroke="#45475a" strokeWidth="10" />
    <rect x="372" y="322" width="28" height="28" fill="#89b4fa" />
  </svg>
);
