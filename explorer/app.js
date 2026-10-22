const ERROR_CODES = {
  0: 'NotAuthorized',
  1: 'AlreadyInitialized',
  2: 'AssetNotRegistered',
  3: 'AssetAlreadyRegistered',
  4: 'SourceAlreadyExists',
  5: 'SourceNotFound',
  6: 'InsufficientSources',
  7: 'InvalidPrice',
  8: 'NoData',
  9: 'InvalidTimestamp',
  10: 'InvalidConfiguration',
  11: 'DescriptionTooLong',
  12: 'ContractPaused',
  13: 'TimelockNotReady',
  14: 'OperationNotFound',
  15: 'PriceBelowMinimum',
  16: 'RateLimitExceeded',
  17: 'InvalidDuration',
  18: 'SubscriptionExpired',
  19: 'MigrationInProgress',
  20: 'NoMigrationInProgress',
  21: 'SourceNameEmpty',
  22: 'SourceNameTooLong',
  23: 'MaxSourcesReached',
  24: 'InvalidOperationType',
  25: 'Reentrant',
  26: 'SourceNotPendingRemoval',
  27: 'CooldownNotElapsed',
  28: 'MaxAssetsReached',
  29: 'ReasonTooLong',
  30: 'RecordsLimitExceeded',
  31: 'CommitHashMismatch',
  32: 'CommitExpired',
  33: 'CommitNotFound',
  34: 'RevealWindowClosed',
  35: 'AlreadyCommitted',
  36: 'InvalidCommitRound',
  37: 'CommitRevealRequired',
  40: 'AlreadyFinalized',
  41: 'PriceRetracted',
  42: 'FinalityPending',
  43: 'InsufficientFinality',
  44: 'ReorgDetected',
  50: 'RelayerNotAuthorized',
  51: 'RelayerAlreadyExists',
  52: 'ReputationTooHighToSlash',
  53: 'MaxSubscriptionsReached',
  57: 'ProposalNotFound',
  58: 'AlreadyApproved',
  59: 'ProposalAlreadyExecuted',
  60: 'InsufficientBond',
  61: 'StakeTokenNotConfigured',
  62: 'SourceSuspended',
  63: 'InvalidDemeritThreshold',
  64: 'InvalidGovernanceConfig',
  65: 'PriceOutOfBounds',
  66: 'AssetPaused',
  67: 'CircuitBreakerTripped',
  75: 'OperationLimitExceeded',
  76: 'ArithmeticOverflow',
  77: 'PoolAlreadyExists',
  78: 'PoolNotFound',
  79: 'SlippageExceeded',
  80: 'AmmPriceManipulation',
  81: 'OutOfSubmissionWindow',
  82: 'ChannelNotFound',
  83: 'ChannelAlreadyOpen',
  84: 'ExoticCycleLimitExceeded',
  85: 'ExoticCycleDetected',
  86: 'ExoticAssetNotConfigured',
  87: 'FeeMarketBelowMinimum',
  88: 'ApprovalNotFound',
  89: 'MsNotQueueHead',
  90: 'MsQuorumNotReached',
  91: 'BondTooSmall',
  92: 'ProposalAlreadyDisputed',
  93: 'ProposalAlreadyResolved',
  94: 'ProposalExpired',
  95: 'ProposalNotDisputed',
  96: 'ZkProofInvalid',
  97: 'ZkVkNotSet',
  98: 'ZkInvalidPublicSignals',
  99: 'SignatureExpired',
  100: 'InvalidNonce',
  101: 'SigningKeyNotRegistered',
  102: 'NotificationConfigInvalid',
  103: 'ExportLimitExceeded',
  104: 'ExportNotFound',
  110: 'InvalidPriority',
  111: 'PriorityTimelockNotReady',
  112: 'InvalidProof',
  113: 'ProofTypeMismatch',
  114: 'TooManyCallbacks',
  115: 'CallbackNotFound',
  116: 'InvalidAssetSelection',
  118: 'PriceNotFrozen',
  119: 'RelayerBondInsufficient',
  120: 'RelayerFailureThresholdNotReached',
  121: 'BatchEmpty',
  122: 'BatchTooLarge',
  123: 'BatchNotFeePrioritized',
};

const METHOD_METADATA = {
  get_price: { kind: 'view', label: 'get_price', args: [
    { name: 'asset', type: 'address', placeholder: 'CDS...' },
    { name: 'max_age', type: 'u64', placeholder: '600' },
  ]},
  get_source_price: { kind: 'view', label: 'get_source_price', args: [
    { name: 'asset', type: 'address', placeholder: 'CDS...' },
    { name: 'source', type: 'address', placeholder: 'G...' },
  ]},
  get_all_prices: { kind: 'view', label: 'get_all_prices', args: [
    { name: 'asset', type: 'address', placeholder: 'CDS...' },
  ]},
  is_source: { kind: 'view', label: 'is_source', args: [
    { name: 'source', type: 'address', placeholder: 'G...' },
  ]},
  is_asset_registered: { kind: 'view', label: 'is_asset_registered', args: [
    { name: 'asset', type: 'address', placeholder: 'CDS...' },
  ]},
  submit_price: { kind: 'invoke', label: 'submit_price', args: [
    { name: 'source', type: 'address', placeholder: 'G...' },
    { name: 'asset', type: 'address', placeholder: 'CDS...' },
    { name: 'price', type: 'i128', placeholder: '1000000' },
    { name: 'timestamp', type: 'u64', placeholder: '1710000000' },
    { name: 'nonce', type: 'u64', placeholder: '1' },
  ]},
  register_asset: { kind: 'invoke', label: 'register_asset', args: [
    { name: 'asset', type: 'address', placeholder: 'CDS...' },
  ]},
  add_source: { kind: 'invoke', label: 'add_source', args: [
    { name: 'source', type: 'address', placeholder: 'G...' },
    { name: 'name', type: 'string', placeholder: 'BTC Feed' },
  ]},
  delegate_relayer: { kind: 'invoke', label: 'delegate_relayer', args: [
    { name: 'source', type: 'address', placeholder: 'G...' },
    { name: 'relayer', type: 'address', placeholder: 'G...' },
    { name: 'nonce', type: 'u64', placeholder: '1' },
    { name: 'expiration_ledger', type: 'u32', placeholder: '200000' },
    { name: 'signature', type: 'bytes', placeholder: 'hex' },
  ]},
};

const dom = {
  rpcUrl: document.getElementById('rpcUrl'),
  contractId: document.getElementById('contractId'),
  networkPassphrase: document.getElementById('networkPassphrase'),
  methodSelect: document.getElementById('methodSelect'),
  modeSelect: document.getElementById('modeSelect'),
  argsContainer: document.getElementById('argsContainer'),
  writeFields: document.getElementById('writeFields'),
  signerSecret: document.getElementById('signerSecret'),
  responseOutput: document.getElementById('responseOutput'),
  errorCodeInput: document.getElementById('errorCodeInput'),
  errorLookupOutput: document.getElementById('errorLookupOutput'),
  callBtn: document.getElementById('callBtn'),
  resetBtn: document.getElementById('resetBtn'),
  errorLookupBtn: document.getElementById('errorLookupBtn'),
};

function populateMethods() {
  const options = Object.entries(METHOD_METADATA)
    .map(([key, value]) => `<option value="${key}">${value.label}</option>`)
    .join('');
  dom.methodSelect.innerHTML = options;
  renderArgs();
}

function renderArgs() {
  const method = dom.methodSelect.value;
  const metadata = METHOD_METADATA[method];
  const mode = dom.modeSelect.value;
  dom.writeFields.classList.toggle('hidden', mode === 'view' || !metadata || metadata.kind !== 'invoke');

  dom.argsContainer.innerHTML = (metadata.args || [])
    .map((arg) => `
      <label>
        ${arg.name}
        <input data-arg="${arg.name}" type="text" placeholder="${arg.placeholder || ''}" />
      </label>
    `)
    .join('');
}

function getArgValues() {
  const method = dom.methodSelect.value;
  const fields = [...document.querySelectorAll('[data-arg]')];
  const metadata = METHOD_METADATA[method];
  const values = {};
  for (const arg of metadata.args) {
    const input = fields.find((field) => field.dataset.arg === arg.name);
    values[arg.name] = input ? input.value.trim() : '';
  }
  return values;
}

function parseValue(raw, type) {
  if (raw === '' || raw === null || raw === undefined) {
    return undefined;
  }
  switch (type) {
    case 'address':
      return StellarSdk.Address.fromString(raw);
    case 'string':
      return raw;
    case 'u64':
    case 'u32':
      return Number(raw);
    case 'i128':
      return BigInt(raw);
    case 'bytes':
      return raw.startsWith('0x') ? raw.slice(2) : raw;
    default:
      return raw;
  }
}

function serializeForDisplay(value) {
  if (typeof value === 'bigint') return value.toString();
  if (Array.isArray(value)) return value.map(serializeForDisplay);
  if (value && typeof value === 'object') {
    return Object.fromEntries(
      Object.entries(value).map(([k, v]) => [k, serializeForDisplay(v)])
    );
  }
  return value;
}

function formatJson(value) {
  return JSON.stringify(serializeForDisplay(value), null, 2);
}

function errorLookup(code) {
  const parsed = Number(code);
  if (!Number.isInteger(parsed)) {
    dom.errorLookupOutput.textContent = 'Enter a valid integer error code.';
    return;
  }
  const label = ERROR_CODES[parsed] || 'UnknownErrorCode';
  dom.errorLookupOutput.textContent = `Code ${parsed}: ${label}`;
}

async function callContract() {
  const rpcUrl = dom.rpcUrl.value.trim() || 'https://soroban-testnet.stellar.org';
  const contractId = dom.contractId.value.trim();
  const method = dom.methodSelect.value;
  const mode = dom.modeSelect.value;

  if (!contractId) {
    dom.responseOutput.textContent = 'Set a valid contract ID before calling the network.';
    return;
  }

  try {
    const server = new StellarSdk.SorobanRpc.Server(rpcUrl, { allowHttp: true });
    const contract = new StellarSdk.Contract(contractId);
    const methodMeta = METHOD_METADATA[method];
    const args = methodMeta.args.map((arg) => parseValue(getArgValues()[arg.name], arg.type));
    const filteredArgs = args.filter((arg) => arg !== undefined);

    if (mode === 'view') {
      const tx = new StellarSdk.TransactionBuilder(new StellarSdk.Account('GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF', '0'), {
        fee: '100',
        networkPassphrase: dom.networkPassphrase.value.trim() || StellarSdk.Networks.TESTNET,
      })
        .addOperation(contract.call(method, ...filteredArgs.map((arg) => StellarSdk.nativeToScVal(arg))))
        .setTimeout(30)
        .build();

      const sim = await server.simulateTransaction(tx);
      if (StellarSdk.SorobanRpc.Api.isSimulationError(sim)) {
        throw new Error(sim.error || 'Simulation error');
      }

      const raw = sim.result?.retval;
      const response = raw ? StellarSdk.scValToNative(raw) : undefined;
      dom.responseOutput.textContent = formatJson(response);
      return;
    }

    const signerSecret = dom.signerSecret.value.trim();
    if (!signerSecret) {
      dom.responseOutput.textContent = 'A signer secret is required for invoke calls.';
      return;
    }

    const signer = StellarSdk.Keypair.fromSecret(signerSecret);
    const account = await server.getAccount(signer.publicKey());
    const tx = new StellarSdk.TransactionBuilder(account, {
      fee: '100000',
      networkPassphrase: dom.networkPassphrase.value.trim() || StellarSdk.Networks.TESTNET,
    })
      .addOperation(contract.call(method, ...filteredArgs.map((arg) => StellarSdk.nativeToScVal(arg))))
      .setTimeout(30)
      .build();

    const prepared = await server.prepareTransaction(tx);
    prepared.sign(signer);
    const sendResult = await server.sendTransaction(prepared);

    let status = sendResult.status;
    let hash = sendResult.hash;
    let result;

    for (let i = 0; i < 10 && status === 'PENDING'; i++) {
      await new Promise((resolve) => setTimeout(resolve, 1000));
      const txResult = await server.getTransaction(hash);
      status = txResult.status;
      if (status === 'SUCCESS') {
        result = txResult.returnValue ? StellarSdk.scValToNative(txResult.returnValue) : null;
        break;
      }
      if (status === 'FAILED') {
        throw new Error(JSON.stringify(txResult, null, 2));
      }
    }

    if (status !== 'SUCCESS') {
      throw new Error(`Transaction status: ${status}`);
    }

    dom.responseOutput.textContent = formatJson(result ?? { status, hash });
  } catch (error) {
    dom.responseOutput.textContent = formatJson({
      error: error instanceof Error ? error.message : String(error),
      request: {
        rpcUrl,
        contractId,
        method,
        mode,
      },
    });
  }
}

dom.methodSelect.addEventListener('change', renderArgs);
dom.modeSelect.addEventListener('change', renderArgs);
dom.errorLookupBtn.addEventListener('click', () => errorLookup(dom.errorCodeInput.value));
dom.callBtn.addEventListener('click', callContract);
dom.resetBtn.addEventListener('click', () => {
  dom.responseOutput.textContent = '{}';
  dom.errorLookupOutput.textContent = 'Enter an error code';
  dom.signerSecret.value = '';
  dom.contractId.value = '';
  dom.methodSelect.value = 'get_price';
  dom.modeSelect.value = 'view';
  renderArgs();
});

populateMethods();
renderArgs();
errorLookup(50);
