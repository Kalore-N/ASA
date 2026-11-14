%global debug_package %{nil}

Name:           asa
Version:        1.6.0
Release:        1%{?dist}
Summary:        去中心化 TUI 群聊
License:        MIT
URL:            https://github.com/Kalore-N/ASA
BuildArch:      x86_64
AutoReqProv:    no
Requires:       libcap

Source0:        asa-1.6.0
Source1:        asa.desktop
Source2:        asa.1.gz
Source3:        asa.png
Source4:        LICENSE
Source5:        README.md
Source6:        easytier-core
Source7:        easytier-cli
Source8:        EASYTIER_LICENSE
Source9:        THIRD_PARTY_NOTICES.md

%description
ASA 是一个完全去中心化的终端群聊工具，支持端到端加密、文件传输、
群固定入口与跨网中继。所有节点对等，没有中心服务器。

%prep

%build

%install
install -Dm755 %{SOURCE0} %{buildroot}%{_bindir}/asa
install -Dm755 %{SOURCE6} %{buildroot}%{_prefix}/lib/asa/easytier-core
install -Dm755 %{SOURCE7} %{buildroot}%{_prefix}/lib/asa/easytier-cli
install -Dm644 %{SOURCE1} %{buildroot}%{_datadir}/applications/asa.desktop
install -Dm644 %{SOURCE2} %{buildroot}%{_mandir}/man1/asa.1.gz
install -Dm644 %{SOURCE3} %{buildroot}%{_datadir}/icons/hicolor/256x256/apps/asa.png
install -Dm644 %{SOURCE4} %{buildroot}%{_datadir}/licenses/asa/LICENSE
install -Dm644 %{SOURCE8} %{buildroot}%{_datadir}/licenses/asa/EASYTIER_LICENSE
install -Dm644 %{SOURCE5} %{buildroot}%{_datadir}/doc/asa/README.md
install -Dm644 %{SOURCE9} %{buildroot}%{_datadir}/doc/asa/THIRD_PARTY_NOTICES.md

%post
if command -v setcap >/dev/null 2>&1; then
  setcap cap_net_admin,cap_net_raw+ep %{_prefix}/lib/asa/easytier-core 2>/dev/null || :
fi

%files
%{_bindir}/asa
%{_prefix}/lib/asa/easytier-core
%{_prefix}/lib/asa/easytier-cli
%{_datadir}/applications/asa.desktop
%{_mandir}/man1/asa.1.gz
%{_datadir}/icons/hicolor/256x256/apps/asa.png
%license %{_datadir}/licenses/asa/LICENSE
%license %{_datadir}/licenses/asa/EASYTIER_LICENSE
%doc %{_datadir}/doc/asa/README.md
%doc %{_datadir}/doc/asa/THIRD_PARTY_NOTICES.md
