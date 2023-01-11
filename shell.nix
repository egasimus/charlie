{ pkgs ? import <nixpkgs> {}, ... }: pkgs.mkShell {
  name = "dawless";
  buildInputs       = with pkgs; [ ncurses wayland libxkbcommon xlibsWrapper xorg.xcbutil xorg.xcbutilimage xorg.libXcursor xorg.libXrandr xorg.libXi libglvnd udev libseat dbus libinput gnome.gdm mesa ];
  nativeBuildInputs = with pkgs; [ pkg-config cmake xwayland mold mesa-demos wezterm ];
  LD_LIBRARY_PATH   = with pkgs; lib.strings.makeLibraryPath [ libglvnd wayland ];
}
