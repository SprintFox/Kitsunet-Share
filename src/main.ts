import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";

// --- DOM Elements ---
const radar = document.querySelector('.radar') as HTMLElement;
const selfDot = document.querySelector('.radar-dot.self') as HTMLElement;
const usernameInput = document.getElementById('username-input') as HTMLInputElement;
const broadcastToggle = document.getElementById('broadcast-toggle') as HTMLInputElement;

// Modals
const fileOfferModal = document.getElementById('file-offer-modal') as HTMLElement;
const settingsModal = document.getElementById('settings-modal') as HTMLElement;
const fileOfferTitle = document.getElementById('file-offer-title') as HTMLElement;
const fileOfferCloseButton = document.getElementById('file-offer-close-button') as HTMLElement;
const incomingFileList = document.getElementById('incoming-file-list') as HTMLElement;

// Buttons
const acceptOfferBtn = document.getElementById('accept-offer-btn') as HTMLButtonElement;
const declineOfferBtn = document.getElementById('decline-offer-btn') as HTMLButtonElement;
const addFileBtn = document.getElementById('add-file-btn') as HTMLButtonElement;
const dynamicSendBtn = document.getElementById('dynamic-send-btn') as HTMLButtonElement;
const settingsBtn = document.getElementById('settings-btn') as HTMLButtonElement;

// File & Transfer UI
const fileList = document.getElementById('file-list') as HTMLElement;

// Settings
const networkInterfaceSelect = document.getElementById('network-interface-select') as HTMLSelectElement;

// --- State ---
let filePathsToSend: string[] = [];
let selectedPeerAddress: string | null = null;
let currentOfferId: string | null = null;
let isTransferring = false;

// --- Functions ---

function setTransferring(state: boolean) {
    isTransferring = state;
    dynamicSendBtn.disabled = state;

    addFileBtn.style.display = state ? 'none' : 'block';
    fileList.classList.toggle('transfer-in-progress', state);

    const peerDots = document.querySelectorAll('.radar-dot.peer');
    if (state) {
        radar.style.pointerEvents = 'none';
        peerDots.forEach(dot => (dot as HTMLElement).style.cursor = 'not-allowed');
        dynamicSendBtn.style.cursor = 'not-allowed';
    } else {
        radar.style.pointerEvents = 'auto';
        peerDots.forEach(dot => (dot as HTMLElement).style.cursor = 'pointer');
        dynamicSendBtn.style.cursor = 'pointer';
    }
}

function formatBytes(bytes: number, decimals = 2) {
    if (bytes === 0) return '0 Bytes';
    const k = 1024;
    const dm = decimals < 0 ? 0 : decimals;
    const sizes = ['Bytes', 'KB', 'MB', 'GB', 'TB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(dm)) + ' ' + sizes[i];
}

function escapeCSSSelector(selector: string): string {
    return selector.replace(/\\/g, '\\\\');
}

async function updatePeerList() {
    if (isTransferring) return;
    const peers = await invoke<{ address: string, username: string }[]>('get_users');
    const ownAddress: string = await invoke('get_own_address');

    document.querySelectorAll('.radar-dot.peer').forEach(dot => dot.remove());

    const otherPeers = peers.filter(p => p.address !== ownAddress);
    const angleStep = 360 / (otherPeers.length || 1);

    otherPeers.forEach((peer, index) => {
        const dot = document.createElement('div');
        dot.className = 'radar-dot peer';
        dot.dataset.peerAddress = peer.address;
        dot.dataset.peerUsername = peer.username;
        dot.textContent = peer.username;
        dot.style.setProperty('--angle', `${index * angleStep}deg`);
        radar.appendChild(dot);
    });
}

async function loadNetworkInterfaces() {
    const interfaces: { name: string, ip: string, broadcast: string }[] = await invoke('get_network_interfaces');
    networkInterfaceSelect.innerHTML = '';
    interfaces.forEach(iface => {
        const option = document.createElement('option');
        option.value = iface.broadcast;
        option.textContent = `${iface.name} (${iface.ip})`;
        networkInterfaceSelect.appendChild(option);
    });
}

function togglePulse(isBroadcasting: boolean) {
    if (isBroadcasting) {
        selfDot.classList.add('pulsating');
    } else {
        selfDot.classList.remove('pulsating');
    }
}

async function loadSettings() {
    await loadNetworkInterfaces();
    const settings: { username: string, broadcasting_enabled: boolean, broadcast_address: string } = await invoke('get_settings');
    usernameInput.value = settings.username;
    broadcastToggle.checked = settings.broadcasting_enabled;
    selfDot.textContent = settings.username;
    networkInterfaceSelect.value = settings.broadcast_address;
    togglePulse(settings.broadcasting_enabled);
}

async function saveSettings() {
    const newSettings = {
        username: usernameInput.value,
        broadcasting_enabled: broadcastToggle.checked,
        broadcast_address: networkInterfaceSelect.value,
    };
    await invoke('update_settings', { settings: newSettings });
    selfDot.textContent = newSettings.username;
    togglePulse(newSettings.broadcasting_enabled);
}

function renderFileList() {
    fileList.innerHTML = '';

    if (filePathsToSend.length === 0) {
        const emptyMessage = document.createElement('li');
        emptyMessage.textContent = 'No files 🌀';
        emptyMessage.style.justifyContent = 'center';
        emptyMessage.style.background = 'transparent';
        fileList.appendChild(emptyMessage);
        return;
    }

    filePathsToSend.forEach(async (path, index) => {
        const listItem = document.createElement('li');
        listItem.dataset.filePath = path;
        const fileNameStr = path.replace(/^.*[\\\/]/, '');

        const fileInfo = document.createElement('div');
        fileInfo.className = 'file-info';

        const previewContainer = document.createElement('div');
        previewContainer.className = 'file-preview';

        const extension = fileNameStr.split('.').pop()!.toLowerCase();
        if (['png', 'jpg', 'jpeg', 'gif', 'webp', 'svg'].includes(extension)) {
            const img = document.createElement('img');
            img.src = convertFileSrc(path);
            previewContainer.appendChild(img);
        } else {
            previewContainer.textContent = extension.substring(0, 4);
        }

        fileInfo.appendChild(previewContainer);

        const fileInfoWrapper = document.createElement('div');
        fileInfoWrapper.classList.add('fileInfoWrapper');

        const fileName = document.createElement('span');
        fileName.className = 'file-name';
        fileName.textContent = fileNameStr;
        fileName.title = fileNameStr;
        fileInfoWrapper.appendChild(fileName);

        const transferDetails = document.createElement('div');
        transferDetails.className = 'transfer-details';

        const progress = document.createElement('progress');
        progress.max = 100;
        progress.value = 0;
        transferDetails.appendChild(progress);

        const statusIcon = document.createElement('span');
        statusIcon.className = 'status-icon';
        transferDetails.appendChild(statusIcon);

        fileInfoWrapper.appendChild(transferDetails);
        fileInfo.appendChild(fileInfoWrapper);
        listItem.appendChild(fileInfo);

        const removeBtn = document.createElement('button');
        removeBtn.className = 'remove-file-btn';
        removeBtn.innerHTML = '&times;';
        removeBtn.addEventListener('click', (e) => {
            if (isTransferring) return;
            e.stopPropagation();
            filePathsToSend.splice(index, 1);
            renderFileList();
        });
        listItem.appendChild(removeBtn);

        fileList.appendChild(listItem);
    });
}

function showModal(modal: HTMLElement) {
    modal.classList.add('visible');
}

function hideModal(modal: HTMLElement) {
    modal.classList.remove('visible');
}

function showFileOffer(offer: { payload: { id: string, from: string, files: { name: string, size: number }[], total_size: number } }) {
    const { id, from, files, total_size } = offer.payload;
    currentOfferId = id;
    fileOfferTitle.textContent = `Incoming transfer from ${from}`;
    fileOfferCloseButton.classList.remove("visible")

    acceptOfferBtn.style.display = 'block';
    declineOfferBtn.style.display = 'block';

    incomingFileList.innerHTML = '';
    files.forEach(file => {
        const li = document.createElement('li');
        li.dataset.fileName = file.name;
        li.innerHTML = `
          <div class="file-info" style="flex-grow: 1;">
              <span class="file-name">${file.name} (${formatBytes(file.size)})</span>
          </div>
          <div class="receiving-details">
              <progress max="100" value="0" style="display: none;"></progress>
              <span class="status-icon"></span>
              <button class="show-in-folder-btn" style="display: none;">🔎</button>
          </div>
        `;
        incomingFileList.appendChild(li);
    });
    const totalLi = document.createElement('li');
    totalLi.innerHTML = `<strong>Total: ${formatBytes(total_size)}</strong>`;
    incomingFileList.appendChild(totalLi);

    showModal(fileOfferModal);
}

// --- Event Listeners ---

settingsBtn.addEventListener('click', () => showModal(settingsModal));

document.querySelectorAll('.close-btn').forEach(btn => {
    btn.addEventListener('click', () => {
        const modalId = (btn as HTMLElement).dataset.closeModal;
        if (modalId) {
            const modal = document.getElementById(modalId);
            if (modal) {
                hideModal(modal);
            }
        }
    });
});

document.querySelectorAll('.modal-overlay').forEach(overlay => {
    overlay.addEventListener('click', (e) => {
        if (e.target === overlay) {
            hideModal(overlay as HTMLElement);
        }
    });
});


addFileBtn.addEventListener('click', async () => {
    if (isTransferring) return;
    const selected = await open({
        multiple: true,
    });
    if (Array.isArray(selected)) {
        filePathsToSend.push(...selected);
        renderFileList();
    } else if (selected) {
        filePathsToSend.push(selected);
        renderFileList();
    }
});

radar.addEventListener('click', (e) => {
    if (isTransferring) return;
    const target = (e.target as HTMLElement).closest('.radar-dot.peer') as HTMLElement;

    if (target && target.classList.contains('selected')) {
        target.classList.remove('selected');
        selectedPeerAddress = null;
        dynamicSendBtn.style.display = 'none';
    } else {
        document.querySelectorAll('.radar-dot.peer.selected').forEach(dot => dot.classList.remove('selected'));

        if (target) {
            target.classList.add('selected');
            selectedPeerAddress = target.dataset.peerAddress!;

            const rect = target.getBoundingClientRect();
            dynamicSendBtn.style.display = 'block';
            dynamicSendBtn.style.top = `${rect.top}px`;
            dynamicSendBtn.style.left = `${rect.left + (rect.width / 2) - (dynamicSendBtn.offsetWidth / 2)}px`;
        } else {
            selectedPeerAddress = null;
            dynamicSendBtn.style.display = 'none';
        }
    }
});

dynamicSendBtn.addEventListener('click', async () => {
    if (isTransferring) return;
    if (filePathsToSend.length === 0) {
        alert('Please select files to send.');
        return;
    }
    if (!selectedPeerAddress) {
        alert('Please select a recipient on the radar.');
        return;
    }

    const recipientAddress = selectedPeerAddress;
    const recipientDot = document.querySelector(`.radar-dot.peer[data-peer-address="${recipientAddress}"]`);

    setTransferring(true);

    if (recipientDot) {
        recipientDot.classList.remove('selected');
        recipientDot.classList.add('transferring');
    }
    selectedPeerAddress = null;
    dynamicSendBtn.style.display = 'none';

    document.querySelectorAll('#file-list .transfer-details').forEach(details => {
        details.classList.add('visible');
    });

    try {
        await invoke('send_files', {
            recipient: recipientAddress,
            filePaths: filePathsToSend,
        });
    } catch (error) {
        console.error(`Failed to send files:`, error);
        alert(`Failed to send files.`);
        // Reset UI on failure
        if (recipientDot) {
            recipientDot.classList.remove('transferring');
        }
        filePathsToSend = [];
        renderFileList();
        setTransferring(false);
    }
});

acceptOfferBtn.addEventListener('click', async () => {
    if (currentOfferId) {
        setTransferring(true);
        // Hide accept/decline buttons and show progress bars
        acceptOfferBtn.style.display = 'none';
        declineOfferBtn.style.display = 'none';
        document.querySelectorAll('#incoming-file-list li').forEach(li => {
            const progressBar = li.querySelector('progress');
            if (progressBar) progressBar.style.display = 'block';
        });

        try {
            await invoke('accept_file_offer', { offerId: currentOfferId });
        } catch (error) {
            console.error('Failed to accept offer:', error);
            alert('Failed to start file reception.');
            setTransferring(false);
            acceptOfferBtn.style.display = 'block';
            declineOfferBtn.style.display = 'block';
        }
    }
});

declineOfferBtn.addEventListener('click', async () => {
    if (currentOfferId) {
        await invoke('reject_file_offer', { offerId: currentOfferId });
        hideModal(fileOfferModal);
    }
});

listen('peers_updated', updatePeerList);
listen('file-offer', showFileOffer);
listen('transfer-progress', (event) => {
    const { file_path, file_name, progress } = event.payload as { file_path: string, file_name: string, progress: number };

    // For sender
    if (file_path) {
        const escapedPath = escapeCSSSelector(file_path);
        const fileLi = document.querySelector(`#file-list li[data-file-path="${escapedPath}"]`);
        if (fileLi) {
            const progressBar = fileLi.querySelector('progress');
            if (progressBar) progressBar.value = progress;
        }
    }

    // For receiver
    if (file_name) {
        const fileLi = document.querySelector(`#incoming-file-list li[data-file-name="${file_name}"]`);
        if (fileLi) {
            const progressBar = fileLi.querySelector('progress');
            if (progressBar) progressBar.value = progress;
        }
    }
});
listen('transfer-complete', (event) => {
    const { file_path, file_name, saved_path } = event.payload as { file_path: string, file_name: string, saved_path: string };

    // For sender
    if (file_path) {
        const escapedPath = escapeCSSSelector(file_path);
        const fileLi = document.querySelector(`#file-list li[data-file-path="${escapedPath}"]`);
        if (fileLi) {
            const statusIcon = fileLi.querySelector('.status-icon');
            if (statusIcon) statusIcon.classList.add('complete');
            const progressBar = fileLi.querySelector('progress');
            if (progressBar) progressBar.style.display = 'none';
        }
    }

    // For receiver
    if (file_name) {
        const fileLi = document.querySelector(`#incoming-file-list li[data-file-name="${file_name}"]`);
        if (fileLi) {
            const statusIcon = fileLi.querySelector('.status-icon');
            if (statusIcon) statusIcon.classList.add('complete');
            const progressBar = fileLi.querySelector('progress');
            if (progressBar) progressBar.style.display = 'none';
            const showBtn = fileLi.querySelector('.show-in-folder-btn') as HTMLButtonElement;
            if (showBtn) {
                showBtn.style.display = 'block';
                showBtn.addEventListener('click', () => {
                    invoke('show_in_folder', { path: saved_path });
                });
            }
        }
    }

    const totalSenderFiles = filePathsToSend.length;
    const completedSenderFiles = document.querySelectorAll('#file-list .status-icon.complete').length;
    const recipientDot = document.querySelector('.radar-dot.peer.transferring');

    if (totalSenderFiles > 0 && completedSenderFiles === totalSenderFiles) {
        filePathsToSend = [];
        setTransferring(false);
        if (recipientDot) {
            recipientDot.classList.remove('transferring');
        }
        setTimeout(() => {
            renderFileList();
        }, 2000);
    }

    const allReceiverFilesDone = [...document.querySelectorAll('#incoming-file-list .status-icon')].every(icon => icon.classList.contains('complete'));
    if (allReceiverFilesDone && currentOfferId) {
        setTransferring(false);
        fileOfferCloseButton.classList.add("visible")
    }
});

usernameInput.addEventListener('input', saveSettings);
broadcastToggle.addEventListener('change', saveSettings);
networkInterfaceSelect.addEventListener('change', saveSettings);

// --- Initial Load ---
updatePeerList();
loadSettings();
renderFileList();