package UpdateableLog;

use UpdateableLogLine;

use strict;

# a log system that allows updates to log lines after logging

sub new {
  my ($class) = @_;

  my $self =
    {
     'list', [],		# a list of log lines by position on screen
    };

  return bless $self, $class;
}

sub sort($) {
  my ($self) = @_;
  $self->clear();
  my @sorted_logs = sort { $a->{value} <=> $b->{value} } @{$self->{list}};
  $self->{list} = \@sorted_logs;
  $self->redraw();
}

sub redraw($) {
  my ($self) = @_;

  my $index = scalar(@{$self->{list}});
  foreach my $line (@{$self->{list}}) {
    print "\033[$index"."A";	# move cursor up $index lines
    print $line->{message};
    print "\n";
    print "\033[$index"."B";  # move cursor down $index lines
    $index--;
  }
}

sub clear($) {
  my ($self) = @_;

  my $index = scalar(@{$self->{list}});
  foreach my $line (@{$self->{list}}) {
    print "\033[$index"."A";	# move cursor up $index lines
    my $length_to_clear = length($line->{message});
    for(my $i = 0 ; $i < $length_to_clear ; $i++) {
      print " ";
    }
    print "\n";
    print "\033[$index"."B";  # move cursor down $index lines
    $index--;
  }
}

sub log($$$) {
  my ($self, $name, $message, $value) = @_;

  my $logline = UpdateableLogLine->new($name, $message, $value);

  my $found = 0;

  my $index = scalar(@{$self->{list}});
  # first look at is it in the list
  foreach my $line (@{$self->{list}}) {
    if($line->{name} eq $name) {
      # update previous log line with new message
      $found = 1;
      my $previous_message_length = length($line->{message});
      my $new_message_length = length($message);
      $line->{message} = $message;
      print "\033[$index"."A";	# move cursor up $index lines
      print $message;
      if($previous_message_length > $new_message_length) {
	my $leftover = $previous_message_length - $new_message_length;
	for(my $i = 0 ; $i < $leftover ; $i++) {
	  print " ";
	}
      }
      print "\n";
      print "\033[$index"."B";  # move cursor down $index lines
    }
    $index--;
  }

  if(!$found) {
    # add to end of list and print it
    push @{$self->{list}}, $logline;

    print "$message\n";
  }
  $self->sort();
}

1;
