package ProcessPipe;

use strict;

# an object that can be used to render an image sequence into a video

sub new {
  my ($class,
      $log,
      $shell_command,
      $group,
      $finished_callback,
     )
      = @_;

  my $self =
    {
     log => $log,
     shell_command => $shell_command,
     is_done => 0,
     is_running => 0,
     group => $group,
     finished_callback => $finished_callback,
    };

  return bless $self, $class;
}

sub group_is_done() {
  my ($self) = @_;

  foreach my $pipe (@{$self->{group}}) {
    return 0 unless $pipe->{is_done};
  }
  return $self->{is_done};
}

sub finish($) {
  my ($self) = @_;

  close $self->{shell_input};

  $self->{is_done} = 1;

  $self->{is_running} = 0;
  if ($self->group_is_done()) {
    # only after all renders are done for this group

    my $finished_callback = $self->{finished_callback};
    &$finished_callback($self);
  }
}

sub read_line($) {
 my ($self) = @_;

 die "should be overwritten by subclasses\n";
}

sub start($) {
  my ($self) = @_;

  return if($self->{is_running} || $self->{is_done});

  my $input_fs;

  open $input_fs, "$self->{shell_command} |" || die "cannot open: $!\n";
  $self->{shell_input} = \*$input_fs;
  $self->{frame_num} = 0;
  $self->{is_running} = 1;
}
1;
