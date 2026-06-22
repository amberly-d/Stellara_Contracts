export enum EventType {
  PAYMENT = 'payment',
  OFFER = 'offer',
  CONTRACT = 'contract',
  ACCOUNT = 'account',
  TRUSTLINE = 'trustline',
}

export enum DeliveryStatus {
  PENDING = 'pending',
  DELIVERED = 'delivered',
  FAILED = 'failed',
  RETRYING = 'retrying',
  DEAD_LETTER = 'dead_letter',
}

export enum ConsumerStatus {
  ACTIVE = 'active',
  INACTIVE = 'inactive',
  SUSPENDED = 'suspended',
}

export interface StellarEventData {
  id: string;
  eventType: EventType;
  ledgerSequence: number;
  timestamp: Date;
  transactionHash: string;
  sourceAccount: string;
  payload: Record<string, any>;
  createdAt: Date;
}

export interface PaymentEventPayload {
  amount: string;
  assetType: string;
  assetCode?: string;
  assetIssuer?: string;
  from: string;
  to: string;
  memo?: string;
}

export interface OfferEventPayload {
  offerId: string;
  seller: string;
  sellingAssetType: string;
  sellingAssetCode?: string;
  sellingAssetIssuer?: string;
  buyingAssetType: string;
  buyingAssetCode?: string;
  buyingAssetIssuer?: string;
  amount: string;
  price: string;
  type: 'create' | 'update' | 'delete';
}

export interface ContractEventPayload {
  contractId: string;
  topics: string[];
  data: string;
  function?: string;
}
