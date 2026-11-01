import { Injectable } from '@nestjs/common';
import { MailService } from '../mail/mail.service';

@Injectable()
export class PaymentsService {
  constructor(private readonly mail: MailService) {}
}
